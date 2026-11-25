package dev.leptoskit.plugins.appshortcuts

import android.content.Context
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.net.Uri
import android.os.Build
import android.os.Looper
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/** Publishes dynamic launcher shortcuts through ShortcutManager and captures incoming ACTION_SEND shares. */
class AppShortcutsPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "app-shortcuts"
    override val methods =
        intArrayOf(
            SET,
            CLEAR,
            CONSUME_LAUNCH,
            INDEX_SPOTLIGHT,
            DELETE_SPOTLIGHT,
            START_USER_ACTIVITY,
            END_USER_ACTIVITY,
            CONSUME_INCOMING_SHARE,
        )

    private var closed = false
    private var launchId: String? = null
    private var pendingShare: IncomingShareData? = null

    init {
        shared = this
    }

    /** Records that the OS launched the app through this shortcut id. */
    fun noteLaunch(id: String) {
        validateId(id)
        launchId = id
    }

    /** Records an incoming `ACTION_SEND` share intent for `consume_incoming_share`. */
    fun noteIncomingShare(intent: Intent?) {
        if (intent == null || intent.action != Intent.ACTION_SEND) return
        val rawText = intent.getStringExtra(Intent.EXTRA_TEXT)?.take(4096)
        val rawSubject = intent.getStringExtra(Intent.EXTRA_SUBJECT)?.take(256)
        val trimmed = rawText?.trim()
        val url =
            if (trimmed != null && (trimmed.startsWith("https://") || trimmed.startsWith("http://"))) {
                trimmed.take(2048)
            } else {
                null
            }
        if (rawText == null && rawSubject == null && url == null) return
        pendingShare = IncomingShareData(text = rawText, url = url, subject = rawSubject)
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        checkMain()

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        return when (method) {
            SET -> {
                set(decodeList(payload))
                byteArrayOf()
            }

            CLEAR -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                clear()
                byteArrayOf()
            }

            CONSUME_LAUNCH -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                encodeLaunch(launchId.also { launchId = null })
            }

            INDEX_SPOTLIGHT -> {
                validateSpotlight(payload)
                throw PluginException(PluginException.UNAVAILABLE)
            }

            DELETE_SPOTLIGHT -> {
                validateSingleId(payload)
                throw PluginException(PluginException.UNAVAILABLE)
            }

            START_USER_ACTIVITY -> {
                validateUserActivity(payload)
                throw PluginException(PluginException.UNAVAILABLE)
            }

            END_USER_ACTIVITY -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                throw PluginException(PluginException.UNAVAILABLE)
            }

            CONSUME_INCOMING_SHARE -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                encodeOptionalShare(pendingShare.also { pendingShare = null })
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }
    }

    override fun close() {
        if (closed) return
        closed = true
        // Best-effort teardown so dynamic shortcuts do not outlive the plugin
        // once SET/CLEAR are refused.
        try {
            clear()
        } catch (_: PluginException) {
        }
    }

    private fun set(items: List<Shortcut>) {
        if (Build.VERSION.SDK_INT < 25) throw PluginException(PluginException.UNAVAILABLE)
        val manager =
            context.getSystemService(ShortcutManager::class.java)
                ?: throw PluginException(PluginException.UNAVAILABLE)
        val launch =
            context.packageManager.getLaunchIntentForPackage(context.packageName)
                ?: throw PluginException(PluginException.UNAVAILABLE)
        val shortcuts =
            items.map { item ->
                val intent =
                    Intent(launch).apply {
                        action = Intent.ACTION_VIEW
                        data = Uri.parse("$SHORTCUT_URI_PREFIX${item.id}")
                        putExtra(EXTRA_ID, item.id)
                        flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
                    }
                ShortcutInfo
                    .Builder(context, item.id)
                    .setShortLabel(item.title)
                    .setLongLabel(item.subtitle.ifEmpty { item.title })
                    .setIntent(intent)
                    .build()
            }
        manager.dynamicShortcuts = shortcuts
    }

    private fun clear() {
        if (Build.VERSION.SDK_INT < 25) throw PluginException(PluginException.UNAVAILABLE)
        val manager =
            context.getSystemService(ShortcutManager::class.java)
                ?: throw PluginException(PluginException.UNAVAILABLE)
        manager.removeAllDynamicShortcuts()
    }

    private data class Shortcut(
        val id: String,
        val title: String,
        val subtitle: String,
    )

    private data class IncomingShareData(
        val text: String?,
        val url: String?,
        val subject: String?,
    )

    private fun decodeList(payload: ByteBuffer): List<Shortcut> {
        val buffer = payload.order(ByteOrder.LITTLE_ENDIAN)

        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val count = buffer.int

        if (count !in 0..8) throw PluginException(PluginException.TOO_LARGE)
        val items = ArrayList<Shortcut>(count)
        val seen = HashSet<String>()
        repeat(count) {
            val id = readString(buffer, 64)
            val title = readString(buffer, 128)
            val subtitle = readString(buffer, 128)
            validateId(id)
            validateText(title, max = 128, allowEmpty = false)
            validateText(subtitle, max = 128, allowEmpty = true)

            if (!seen.add(id)) throw PluginException(PluginException.INVALID_ARGUMENT)
            items.add(Shortcut(id, title, subtitle))
        }

        if (buffer.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return items
    }

    private fun validateSingleId(payload: ByteBuffer) {
        val buffer = payload.order(ByteOrder.LITTLE_ENDIAN)
        val id = readString(buffer, 64)
        validateId(id)
        if (buffer.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    private fun validateSpotlight(payload: ByteBuffer) {
        val buffer = payload.order(ByteOrder.LITTLE_ENDIAN)
        if (buffer.remaining() < 4 || buffer.int != 1) throw PluginException(PluginException.INVALID_ARGUMENT)
        validateId(readString(buffer, 64))
        validateId(readString(buffer, 64))
        validateText(readString(buffer, 128), max = 128, allowEmpty = false)
        validateText(readString(buffer, 1024), max = 1024, allowEmpty = true)
        validateSafeUrl(readString(buffer, 2048))
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val keywords = buffer.int
        if (keywords !in 0..16) throw PluginException(PluginException.TOO_LARGE)
        repeat(keywords) {
            validateText(readString(buffer, 64), max = 64, allowEmpty = false)
        }
        if (buffer.remaining() < 12) throw PluginException(PluginException.INVALID_ARGUMENT)
        val hasExp = buffer.int
        val lo = buffer.int
        val hi = buffer.int
        when (hasExp) {
            0 -> if (lo != 0 || hi != 0) throw PluginException(PluginException.INVALID_ARGUMENT)
            1 -> Unit
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        if (buffer.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    private fun validateUserActivity(payload: ByteBuffer) {
        val buffer = payload.order(ByteOrder.LITTLE_ENDIAN)
        if (buffer.remaining() < 4 || buffer.int != 1) throw PluginException(PluginException.INVALID_ARGUMENT)
        validateId(readString(buffer, 64))
        validateText(readString(buffer, 128), max = 128, allowEmpty = false)
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        when (buffer.int) {
            0 -> {}

            1 -> {
                val url = readString(buffer, 2048)
                if (!url.startsWith("https://") || url.any { Character.isISOControl(it) }) {
                    throw PluginException(PluginException.INVALID_ARGUMENT)
                }
            }

            else -> {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        }
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val flags = buffer.int
        if ((flags and 0b111.inv()) != 0) throw PluginException(PluginException.INVALID_ARGUMENT)
        if (buffer.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    private fun encodeLaunch(id: String?): ByteArray {
        val value = id ?: ""
        if (value.isNotEmpty()) validateId(value)
        val bytes = value.toByteArray(StandardCharsets.UTF_8)
        val out = ByteBuffer.allocate(4 + bytes.size).order(ByteOrder.LITTLE_ENDIAN)
        out.putInt(bytes.size)
        out.put(bytes)

        return out.array()
    }

    private fun encodeOptionalShare(share: IncomingShareData?): ByteArray {
        val out = ByteArrayOutputStream()

        fun writeU32(value: Int) {
            out.write(
                ByteBuffer
                    .allocate(4)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(value)
                    .array(),
            )
        }

        fun writeOptString(value: String?) {
            if (value == null) {
                writeU32(0)
            } else {
                val bytes = value.toByteArray(StandardCharsets.UTF_8)
                writeU32(1)
                writeU32(bytes.size)
                out.write(bytes)
            }
        }
        if (share == null) {
            writeU32(0)
            return out.toByteArray()
        }
        writeU32(1) // Some
        writeU32(1) // version
        writeOptString(share.text)
        writeOptString(share.url)
        writeOptString(share.subject)
        writeU32(0) // attachments
        return out.toByteArray()
    }

    private fun readString(
        buffer: ByteBuffer,
        max: Int,
    ): String {
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val length = buffer.int

        if (length !in 0..max || buffer.remaining() < length) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        val bytes = ByteArray(length)
        buffer.get(bytes)
        val decoder =
            StandardCharsets.UTF_8
                .newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)

        return try {
            decoder.decode(ByteBuffer.wrap(bytes)).toString()
        } catch (_: CharacterCodingException) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    private fun validateId(id: String) {
        if (id.isEmpty() || id.length > 64) throw PluginException(PluginException.INVALID_ARGUMENT)

        if (!id.all { it.isLetterOrDigit() || it == '_' || it == '-' || it == '.' }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    private fun validateText(
        value: String,
        max: Int,
        allowEmpty: Boolean,
    ) {
        if ((!allowEmpty && value.trim().isEmpty()) || value.length > max) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        if (value.any { Character.isISOControl(it) }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    private fun validateSafeUrl(url: String) {
        if (url.isEmpty() || url.length > 2048 || url.any { Character.isISOControl(it) } || !url.contains("://")) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        val lower = url.lowercase()
        for (blocked in listOf("javascript:", "data:", "file:", "blob:", "vbscript:")) {
            if (lower.startsWith(blocked)) throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    private fun checkMain() {
        check(Looper.myLooper() == Looper.getMainLooper())
    }

    companion object {
        const val EXTRA_ID = "dev.leptoskit.plugins.appshortcuts.ID"
        const val SHORTCUT_URI_PREFIX = "leptoskit://app-shortcut/"

        private const val SET = 0
        private const val CLEAR = 1
        private const val CONSUME_LAUNCH = 2
        private const val INDEX_SPOTLIGHT = 3
        private const val DELETE_SPOTLIGHT = 4
        private const val START_USER_ACTIVITY = 5
        private const val END_USER_ACTIVITY = 6
        private const val CONSUME_INCOMING_SHARE = 7

        @Volatile
        var shared: AppShortcutsPlugin? = null
            private set

        /** Notes a launcher shortcut or incoming `ACTION_SEND` share when a plugin instance is registered. */
        @JvmStatic
        fun noteLaunchIfPresent(intent: Intent?) {
            noteIncomingShareIfPresent(intent)
            val id = intent?.getStringExtra(EXTRA_ID) ?: return
            try {
                shared?.noteLaunch(id)
            } catch (_: PluginException) {
            }
        }

        /** Notes an incoming `ACTION_SEND` share when a plugin instance is registered. */
        @JvmStatic
        fun noteIncomingShareIfPresent(intent: Intent?) {
            try {
                shared?.noteIncomingShare(intent)
            } catch (_: PluginException) {
            }
        }
    }
}
