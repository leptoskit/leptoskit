package dev.leptoskit.plugins.homewidgets

import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Looper
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/** Decoded widget snapshot for `AppWidgetProvider` and Jetpack Glance targets. */
data class WidgetSnapshotData(
    val kind: String,
    val family: Int,
    val title: String,
    val body: String,
    val url: String,
)

/** Decoded Android Quick Settings tile state for `TileService` subclasses. */
data class QuickTileData(
    val id: String,
    val label: String,
    val subtitle: String,
    val iconName: String,
    /** `0` = Unavailable, `1` = Inactive, `2` = Active. */
    val state: Int,
    val actionId: String,
)

/**
 * Publishes snapshot, timeline, and Quick Settings tile bytes for Android App
 * Widgets / Jetpack Glance / `TileService` and asks AppWidgetManager to refresh
 * providers. Does not declare or draw a widget provider; the app still owns its
 * `AppWidgetProvider` and `TileService` subclasses.
 */
class HomeWidgetsPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "home-widgets"
    override val methods =
        intArrayOf(
            CAPABILITY,
            SET,
            GET,
            CLEAR,
            RELOAD,
            INSTALLED,
            SET_TIMELINE,
            GET_TIMELINE,
            SET_CONTROL,
            GET_CONTROL,
            RELOAD_CONTROLS,
            SET_QUICK_TILE,
            GET_QUICK_TILE,
        )

    private val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private var closed = false

    override fun close() {
        closed = true
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper())

        if (closed) throw PluginException(PluginException.CANCELLED)
        currentCoroutineContext().ensureActive()

        return when (method) {
            CAPABILITY -> {
                empty(payload)
                capability()
            }

            SET -> {
                val snapshot = Snapshot.decode(payload)
                writeBytes(PREFIX + snapshot.kind, snapshot.raw)
                byteArrayOf()
            }

            GET -> {
                val kind = Kind.decode(payload)
                readBytes(PREFIX + kind)
            }

            CLEAR -> {
                val kind = Kind.decode(payload)

                if (!prefs.edit().remove(PREFIX + kind).commit()) {
                    throw PluginException(PluginException.NATIVE)
                }

                byteArrayOf()
            }

            RELOAD -> {
                val kind = optionalKind(payload)
                reload(kind)
                byteArrayOf()
            }

            INSTALLED -> {
                val kind = Kind.decode(payload)
                installed(kind).toByteArray()
            }

            SET_TIMELINE -> {
                val timeline = TimelinePayload.decode(payload)
                writeBytes(TIMELINE_PREFIX + timeline.kind, timeline.raw)
                byteArrayOf()
            }

            GET_TIMELINE -> {
                val kind = Kind.decode(payload)
                readBytes(TIMELINE_PREFIX + kind)
            }

            SET_CONTROL -> {
                ControlPayload.decode(payload)
                throw PluginException(PluginException.UNAVAILABLE)
            }

            GET_CONTROL -> {
                Kind.decode(payload)
                throw PluginException(PluginException.UNAVAILABLE)
            }

            RELOAD_CONTROLS -> {
                optionalKind(payload)
                throw PluginException(PluginException.UNAVAILABLE)
            }

            SET_QUICK_TILE -> {
                val tile = QuickTilePayload.decode(payload)
                writeBytes(TILE_PREFIX + tile.data.id, tile.raw)
                byteArrayOf()
            }

            GET_QUICK_TILE -> {
                val id = Kind.decode(payload)
                readBytes(TILE_PREFIX + id)
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }
    }

    private fun writeBytes(
        key: String,
        raw: ByteArray,
    ) {
        val encoded = android.util.Base64.encodeToString(raw, android.util.Base64.NO_WRAP)
        // commit() so a successful write is durable before GET/RELOAD.
        if (!prefs.edit().putString(key, encoded).commit()) {
            throw PluginException(PluginException.NATIVE)
        }
    }

    private fun readBytes(key: String): ByteArray {
        val encoded =
            prefs.getString(key, null)
                ?: throw PluginException(PluginException.UNAVAILABLE)
        return android.util.Base64.decode(encoded, android.util.Base64.NO_WRAP)
    }

    private fun capability(): ByteArray {
        // supported | can_reload | can_query_installed | shared_store
        val flags = 1 or (1 shl 1) or (1 shl 2) or (1 shl 3)

        return ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(flags)
            .array()
    }

    private fun reload(kind: String?) {
        val manager = AppWidgetManager.getInstance(context) ?: throw PluginException(PluginException.UNAVAILABLE)
        val providers = providers()

        if (providers.isEmpty()) throw PluginException(PluginException.UNAVAILABLE)

        for (provider in providers) {
            if (kind != null && !provider.className.contains(kind, ignoreCase = true)) {
                // Kind filtering is best-effort by provider class name; apps that
                // need exact mapping should name providers after their kinds.
                continue
            }

            val ids = manager.getAppWidgetIds(provider)

            if (ids.isEmpty()) continue

            val intent =
                Intent(AppWidgetManager.ACTION_APPWIDGET_UPDATE).apply {
                    component = provider
                    putExtra(AppWidgetManager.EXTRA_APPWIDGET_IDS, ids)
                }
            context.sendBroadcast(intent)
        }

        if (kind != null) {
            val matched =
                providers.any { provider ->
                    provider.className.contains(kind, ignoreCase = true) &&
                        manager.getAppWidgetIds(provider).isNotEmpty()
                }
            // Reload with an unknown kind is still accepted when no providers match:
            // the snapshot may be waiting for a future pin.
            if (!matched && providers.none { it.className.contains(kind, ignoreCase = true) }) {
                // Fall through: still accepted. Snapshot storage is the source of truth.
            }
        }
    }

    private fun installed(kind: String): Int {
        val manager = AppWidgetManager.getInstance(context) ?: return INSTALLED_UNKNOWN
        val providers = providers()

        if (providers.isEmpty()) return INSTALLED_NONE
        // Unmatched kinds report None — never fall back to every provider's pins.
        val matching = providers.filter { it.className.contains(kind, ignoreCase = true) }

        if (matching.isEmpty()) return INSTALLED_NONE

        return if (matching.any { manager.getAppWidgetIds(it).isNotEmpty() }) {
            INSTALLED_SOME
        } else {
            INSTALLED_NONE
        }
    }

    private fun providers(): List<ComponentName> {
        val intent = Intent(AppWidgetManager.ACTION_APPWIDGET_UPDATE).setPackage(context.packageName)
        val flags =
            if (android.os.Build.VERSION.SDK_INT >= 33) {
                PackageManager.ResolveInfoFlags.of(PackageManager.GET_RECEIVERS.toLong())
            } else {
                null
            }

        val resolved =
            if (flags != null) {
                context.packageManager.queryBroadcastReceivers(intent, flags)
            } else {
                @Suppress("DEPRECATION")
                context.packageManager.queryBroadcastReceivers(intent, PackageManager.GET_RECEIVERS)
            }
        return resolved.mapNotNull { info ->
            val receiver = info.activityInfo ?: return@mapNotNull null
            ComponentName(receiver.packageName, receiver.name)
        }
    }

    companion object {
        private const val CAPABILITY = 0
        private const val SET = 1
        private const val GET = 2
        private const val CLEAR = 3
        private const val RELOAD = 4
        private const val INSTALLED = 5
        private const val SET_TIMELINE = 6
        private const val GET_TIMELINE = 7
        private const val SET_CONTROL = 8
        private const val GET_CONTROL = 9
        private const val RELOAD_CONTROLS = 10
        private const val SET_QUICK_TILE = 11
        private const val GET_QUICK_TILE = 12
        const val PREFS = "leptoskit.home_widgets"
        const val PREFIX = "leptoskit.home-widgets."
        const val TIMELINE_PREFIX = "leptoskit.home-widgets.timeline."
        const val TILE_PREFIX = "leptoskit.home-widgets.tile."
        private const val INSTALLED_UNKNOWN = 0
        private const val INSTALLED_NONE = 1
        private const val INSTALLED_SOME = 2

        /** Reads the latest widget snapshot stored for `kind`. */
        @JvmStatic
        fun readSnapshot(
            context: Context,
            kind: String,
        ): WidgetSnapshotData? {
            val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val encoded = prefs.getString(PREFIX + kind, null) ?: return null
            return try {
                val raw = android.util.Base64.decode(encoded, android.util.Base64.NO_WRAP)
                Snapshot.decode(ByteBuffer.wrap(raw)).data
            } catch (_: Throwable) {
                null
            }
        }

        /** Reads the latest Quick Settings tile state stored for `id`. */
        @JvmStatic
        fun readQuickTile(
            context: Context,
            id: String,
        ): QuickTileData? {
            val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val encoded = prefs.getString(TILE_PREFIX + id, null) ?: return null
            return try {
                val raw = android.util.Base64.decode(encoded, android.util.Base64.NO_WRAP)
                QuickTilePayload.decode(ByteBuffer.wrap(raw)).data
            } catch (_: Throwable) {
                null
            }
        }
    }
}

private fun empty(payload: ByteBuffer) {
    if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
}

/** HTTP(S) or an app deep-link scheme; rejects javascript/data/file/blob/vbscript. */
private fun isAllowedUrl(url: String): Boolean {
    if (url.any { it.code < 32 || it.code == 127 }) return false
    val separator = url.indexOf(':')

    if (separator <= 0 || separator == url.lastIndex) return false
    val scheme = url.substring(0, separator).lowercase()

    if (scheme[0] !in 'a'..'z') return false

    if (!scheme.drop(1).all { it in 'a'..'z' || it in '0'..'9' || it == '+' || it == '-' || it == '.' }) {
        return false
    }

    return when (scheme) {
        "javascript", "data", "file", "blob", "vbscript" -> false
        else -> true
    }
}

private fun optionalKind(payload: ByteBuffer): String? {
    if (!payload.hasRemaining()) return null
    return Kind.decode(payload)
}

private fun Int.toByteArray(): ByteArray =
    ByteBuffer
        .allocate(4)
        .order(ByteOrder.LITTLE_ENDIAN)
        .putInt(this)
        .array()

private fun sliceRaw(
    payload: ByteBuffer,
    start: Int,
    end: Int,
): ByteArray {
    val raw = ByteArray(end - start)
    payload.duplicate().also {
        it.position(start)
        it.limit(end)
        it.get(raw)
    }
    return raw
}

private object Kind {
    fun decode(payload: ByteBuffer): String {
        val reader = Reader(payload, 68)
        val kind = reader.identifier()
        reader.finish()

        return kind
    }
}

private class Snapshot(
    val kind: String,
    val data: WidgetSnapshotData,
    val raw: ByteArray,
) {
    companion object {
        fun decode(payload: ByteBuffer): Snapshot {
            val start = payload.position()
            val reader = Reader(payload, MAX_SNAPSHOT)
            val kind = reader.identifier()
            val family = reader.u32()

            if (family !in 0..4) throw PluginException(PluginException.INVALID_ARGUMENT)
            val title = reader.text(256, multiline = false, required = true)
            val body = reader.text(1024, multiline = true, required = false)
            val url = reader.text(2048, multiline = false, required = false)

            if (url.isNotEmpty() && !isAllowedUrl(url)) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }

            reader.finish()
            val end = payload.position()
            val raw = sliceRaw(payload, start, end)

            return Snapshot(kind, WidgetSnapshotData(kind, family, title, body, url), raw)
        }

        const val MAX_SNAPSHOT = 16 + 64 + 256 + 1024 + 2048
    }
}

private class TimelinePayload(
    val kind: String,
    val raw: ByteArray,
) {
    companion object {
        fun decode(payload: ByteBuffer): TimelinePayload {
            val start = payload.position()
            val reader = Reader(payload, 32 * 4096)
            if (reader.u32() != 1) throw PluginException(PluginException.INVALID_ARGUMENT)
            val surface = reader.u32()
            if (surface !in 0..6) throw PluginException(PluginException.INVALID_ARGUMENT)
            val policyTag = reader.u32()
            val policySeconds = reader.u32()
            when (policyTag) {
                0, 2 -> if (policySeconds != 0) throw PluginException(PluginException.INVALID_ARGUMENT)
                1 -> if (policySeconds <= 0) throw PluginException(PluginException.INVALID_ARGUMENT)
                else -> throw PluginException(PluginException.INVALID_ARGUMENT)
            }
            val count = reader.u32()
            if (count <= 0) throw PluginException(PluginException.INVALID_ARGUMENT)
            if (count > 32) throw PluginException(PluginException.TOO_LARGE)
            var expectedKind: String? = null
            var prevEpoch = 0L
            for (index in 0 until count) {
                val lo = reader.u32().toLong() and 0xFFFF_FFFFL
                val hi = reader.u32().toLong() and 0xFFFF_FFFFL
                val epoch = lo or (hi shl 32)
                if (index > 0 && java.lang.Long.compareUnsigned(epoch, prevEpoch) < 0) {
                    throw PluginException(PluginException.INVALID_ARGUMENT)
                }
                prevEpoch = epoch
                val snapBytes = reader.rawBytes(Snapshot.MAX_SNAPSHOT)
                val snap = Snapshot.decode(ByteBuffer.wrap(snapBytes))
                if (expectedKind == null) {
                    expectedKind = snap.kind
                } else if (expectedKind != snap.kind) {
                    throw PluginException(PluginException.INVALID_ARGUMENT)
                }
            }
            reader.finish()
            val raw = sliceRaw(payload, start, payload.position())
            return TimelinePayload(expectedKind!!, raw)
        }
    }
}

private object ControlPayload {
    fun decode(payload: ByteBuffer) {
        val reader = Reader(payload, 2048)
        if (reader.u32() != 1) throw PluginException(PluginException.INVALID_ARGUMENT)
        reader.identifier()
        reader.text(128, multiline = false, required = true)
        reader.text(128, multiline = false, required = false)
        reader.identifier()
        val kindTag = reader.u32()
        if (kindTag !in 0..2) throw PluginException(PluginException.INVALID_ARGUMENT)
        reader.identifier()
        reader.finish()
    }
}

private class QuickTilePayload(
    val data: QuickTileData,
    val raw: ByteArray,
) {
    companion object {
        fun decode(payload: ByteBuffer): QuickTilePayload {
            val start = payload.position()
            val reader = Reader(payload, 2048)
            if (reader.u32() != 1) throw PluginException(PluginException.INVALID_ARGUMENT)
            val id = reader.identifier()
            val label = reader.text(128, multiline = false, required = true)
            val subtitle = reader.text(128, multiline = false, required = false)
            val iconName = reader.identifier()
            val state = reader.u32()
            if (state !in 0..2) throw PluginException(PluginException.INVALID_ARGUMENT)
            val actionId = reader.identifier()
            reader.finish()
            val raw = sliceRaw(payload, start, payload.position())
            return QuickTilePayload(
                QuickTileData(id, label, subtitle, iconName, state, actionId),
                raw,
            )
        }
    }
}

private class Reader(
    private val buffer: ByteBuffer,
    maximum: Int,
) {
    private val start = buffer.position()
    private val endExclusive = start + buffer.remaining()

    init {
        if (buffer.remaining() > maximum) throw PluginException(PluginException.INVALID_ARGUMENT)
        buffer.order(ByteOrder.LITTLE_ENDIAN)
    }

    fun u32(): Int {
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)

        return buffer.int
    }

    fun rawBytes(maximum: Int): ByteArray {
        val length = u32()
        if (length < 0 || length > maximum || length > buffer.remaining()) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        val bytes = ByteArray(length)
        buffer.get(bytes)
        return bytes
    }

    fun identifier(): String {
        val value = text(64, multiline = false, required = true)

        if (!value.all { it.isLetterOrDigit() || it == '-' || it == '_' || it == '.' }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return value
    }

    fun text(
        maximum: Int,
        multiline: Boolean,
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
        if (value.any { ch ->
                val code = ch.code
                (code <= 31 || code in 127..159) && !(multiline && (code == 10 || code == 9))
            }
        ) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return value
    }

    fun finish() {
        if (buffer.position() != endExclusive) throw PluginException(PluginException.INVALID_ARGUMENT)
        // Silence unused start warning in some Kotlin versions.
        if (start > endExclusive) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}
