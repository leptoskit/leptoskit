package dev.leptoskit.plugins.sharing

import dev.leptoskit.bridge.PluginException
import java.net.URI
import java.net.URISyntaxException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

/** One file to share: a path inside the app's files directory, or guest bytes. */
internal class ShareFile(
    val path: String?,
    val bytes: ByteArray?,
    val mime: String?,
    val name: String?,
) {
    override fun equals(other: Any?): Boolean =
        other is ShareFile &&
            path == other.path &&
            (bytes?.contentEquals(other.bytes) ?: (other.bytes == null)) &&
            mime == other.mime &&
            name == other.name

    override fun hashCode(): Int = listOf(path, bytes?.contentHashCode(), mime, name).hashCode()
}

/** The popover anchor; Android ignores it after validation. */
internal data class ShareAnchor(
    val x: Float,
    val y: Float,
    val width: Float,
    val height: Float,
)

/** A decoded, validated share request. Every rule matches the Rust and Swift decoders. */
internal data class ShareContent(
    val text: String?,
    val url: String?,
    val subject: String?,
    val title: String?,
    val anchor: ShareAnchor?,
    val excluded: List<Int>,
    val files: List<ShareFile>,
) {
    companion object {
        /** Decodes an untrusted guest payload, refusing unknown versions, codes and trailing data. */
        fun decode(payload: ByteBuffer): ShareContent {
            if (payload.remaining() > MAX_REQUEST_BYTES) throw PluginException(PluginException.TOO_LARGE)

            val reader = WireReader(payload.duplicate().order(ByteOrder.LITTLE_ENDIAN))

            if (reader.u32() != VERSION) throw invalid()

            val text = reader.optionalString()?.also(::validateText)
            val url = reader.optionalString()?.also(::validateUrl)
            val subject = reader.optionalString()?.also(::validateLabel)
            val title = reader.optionalString()?.also(::validateLabel)
            val content =
                ShareContent(text, url, subject, title, anchor(reader), excluded(reader), files(reader))
            reader.finish()

            if (url != null && (text != null || content.files.isNotEmpty())) throw invalid()

            if (text == null && url == null && content.files.isEmpty()) throw invalid()

            return content
        }

        private fun anchor(reader: WireReader): ShareAnchor? =
            when (reader.u32()) {
                0 -> {
                    null
                }

                1 -> {
                    val values = List(ANCHOR_VALUES) { Float.fromBits(reader.u32()) }

                    if (values.any { !it.isFinite() || it < 0f || it > MAX_ANCHOR }) throw invalid()

                    ShareAnchor(values[0], values[1], values[2], values[3])
                }

                else -> {
                    throw invalid()
                }
            }

        private fun excluded(reader: WireReader): List<Int> {
            val count = reader.count(ACTIVITY_COUNT)
            val codes = mutableListOf<Int>()

            repeat(count) {
                val code = reader.u32()

                if (code !in 0 until ACTIVITY_COUNT || code in codes) throw invalid()

                codes += code
            }

            return codes
        }

        private fun files(reader: WireReader): List<ShareFile> {
            val count = reader.count(MAX_FILES)
            val files = mutableListOf<ShareFile>()
            var inline = 0

            repeat(count) {
                val file = file(reader)
                inline += file.bytes?.size ?: 0

                if (inline > MAX_INLINE_BYTES) throw PluginException(PluginException.TOO_LARGE)

                files += file
            }

            return files
        }

        private fun file(reader: WireReader): ShareFile {
            val source = reader.u32()
            val payload = reader.field()
            val mime = reader.optionalString()?.also(SharePaths::validateMime)
            val name = reader.optionalString()?.also(SharePaths::validateName)

            return when (source) {
                APP -> {
                    ShareFile(strict(payload).also(SharePaths::validatePath), null, mime, name)
                }

                BYTES -> {
                    if (payload.size > MAX_INLINE_BYTES) throw PluginException(PluginException.TOO_LARGE)

                    if (name == null) throw invalid()

                    ShareFile(null, payload, mime, name)
                }

                else -> {
                    throw invalid()
                }
            }
        }

        private fun validateText(value: String) {
            if (value.toByteArray(Charsets.UTF_8).size > MAX_TEXT_BYTES) {
                throw PluginException(PluginException.TOO_LARGE)
            }

            if ('\u0000' in value) throw invalid()
        }

        /** A subject or title: bounded UTF-8 without control characters. */
        private fun validateLabel(value: String) {
            if (value.toByteArray(Charsets.UTF_8).size > MAX_LABEL_BYTES) {
                throw PluginException(PluginException.TOO_LARGE)
            }

            if (SharePaths.hasControl(value)) throw invalid()
        }

        /** Accepts an absolute http(s) URL with a visible-ASCII body and no credentials. */
        private fun validateUrl(value: String) {
            if (value.length > MAX_URL_BYTES) throw PluginException(PluginException.TOO_LARGE)

            requireVisible(value)
            val uri = parse(value)

            if (uri.scheme !in WEB_SCHEMES) throw invalid()

            requireDestination(uri)
        }

        /** A URL needs a host, and must not smuggle credentials past its authority. */
        private fun requireDestination(uri: URI) {
            if (uri.host.isNullOrEmpty() || uri.rawAuthority.isNullOrEmpty()) throw invalid()

            if ('@' in uri.rawAuthority || uri.userInfo != null) throw invalid()

            if (uri.port !in -1..MAX_PORT) throw invalid()
        }

        /** Only printable ASCII may appear literally, so no control hides in the value. */
        private fun requireVisible(value: String) {
            if (value.any { it.code !in FIRST_PRINTABLE..LAST_PRINTABLE || it == '\\' }) throw invalid()
        }

        private fun parse(value: String): URI =
            try {
                URI(value)
            } catch (error: URISyntaxException) {
                throw PluginException(PluginException.INVALID_ARGUMENT, error)
            }

        /** Decodes strict UTF-8, refusing bytes that are not text. */
        fun strict(bytes: ByteArray): String =
            try {
                Charsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(ByteBuffer.wrap(bytes))
                    .toString()
            } catch (error: CharacterCodingException) {
                throw PluginException(PluginException.INVALID_ARGUMENT, error)
            }

        fun invalid() = PluginException(PluginException.INVALID_ARGUMENT)

        /** The version the request and reply start with. */
        const val VERSION = 1

        /** File source codes, matching the wire contract. */
        private const val APP = 0
        private const val BYTES = 1

        /** Bounds shared with the Rust crate. */
        private const val MAX_TEXT_BYTES = 65536
        private const val MAX_URL_BYTES = 8192
        private const val MAX_LABEL_BYTES = 1024
        private const val MAX_FILES = 32
        const val MAX_INLINE_BYTES = 4 * 1024 * 1024
        const val MAX_REQUEST_BYTES = 4_418_112
        private const val ACTIVITY_COUNT = 20
        private const val ANCHOR_VALUES = 4
        private const val MAX_ANCHOR = 1_000_000f

        /** Only printable ASCII may appear literally in a shared URL. */
        private const val FIRST_PRINTABLE = 33
        private const val LAST_PRINTABLE = 126

        /** The highest port a URL may name. */
        private const val MAX_PORT = 65535

        private val WEB_SCHEMES = setOf("http", "https")
    }
}

/** Little-endian u32 fields and u32-length-prefixed byte strings. */
internal class WireReader(
    private val bytes: ByteBuffer,
) {
    fun u32(): Int {
        if (bytes.remaining() < Int.SIZE_BYTES) throw ShareContent.invalid()

        return bytes.int
    }

    /** A count no larger than [limit]; larger counts are too large, not malformed. */
    fun count(limit: Int): Int {
        val count = u32()

        if (count !in 0..limit) throw PluginException(PluginException.TOO_LARGE)

        return count
    }

    fun field(): ByteArray {
        val size = u32()

        if (size < 0 || size > bytes.remaining()) throw ShareContent.invalid()

        return ByteArray(size).also { bytes.get(it) }
    }

    /** Strict UTF-8; an empty value means absent. */
    fun optionalString(): String? = field().takeIf { it.isNotEmpty() }?.let(ShareContent::strict)

    fun finish() {
        if (bytes.hasRemaining()) throw ShareContent.invalid()
    }
}

/** The reply: version, status (0 success, 1 dismissed, 2 unavailable), raw identifier. */
internal object ShareReply {
    private const val SUCCESS = 0
    private const val DISMISSED_STATUS = 1
    private const val UNAVAILABLE_STATUS = 2
    private const val FRAMING_BYTES = 12

    /** The largest raw identifier the guest accepts. */
    private const val MAX_RAW_BYTES = 1024

    fun dismissed(): ByteArray = encode(DISMISSED_STATUS, "")

    fun unavailable(): ByteArray = encode(UNAVAILABLE_STATUS, "")

    /** A chosen destination; an identifier the guest would refuse becomes unavailable. */
    fun success(raw: String): ByteArray {
        val usable =
            raw.isNotEmpty() && raw.toByteArray(Charsets.UTF_8).size <= MAX_RAW_BYTES && !SharePaths.hasControl(raw)

        return if (usable) encode(SUCCESS, raw) else unavailable()
    }

    private fun encode(
        status: Int,
        raw: String,
    ): ByteArray {
        val bytes = raw.toByteArray(Charsets.UTF_8)

        return ByteBuffer
            .allocate(FRAMING_BYTES + bytes.size)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(ShareContent.VERSION)
            .putInt(status)
            .putInt(bytes.size)
            .put(bytes)
            .array()
    }
}

/** Path, name and MIME rules shared with the Rust and Swift decoders. */
internal object SharePaths {
    private const val MAX_PATH_BYTES = 4096
    private const val MAX_NAME_BYTES = 255
    private const val MAX_MIME_BYTES = 255
    private const val MAX_MIME_NAME = 127
    private const val DEVICE_NAME_LENGTH = 4
    private const val TEMP_PREFIX = ".leptoskit-tmp-"
    private const val RESERVED = "\\:<>\"|?*"
    private const val MIME_PUNCTUATION = "!#$&-^_.+"
    private val DEVICES = setOf("CON", "PRN", "AUX", "NUL")

    /** A relative path inside the app's files directory, with the filesystem plugin's rules. */
    fun validatePath(path: String) {
        if (path.toByteArray(Charsets.UTF_8).size > MAX_PATH_BYTES) throw PluginException(PluginException.TOO_LARGE)

        if (path.isEmpty() || !path.split('/').all(::component)) throw ShareContent.invalid()
    }

    /** A display name: one path segment. */
    fun validateName(name: String) {
        if (name.toByteArray(Charsets.UTF_8).size > MAX_NAME_BYTES) throw PluginException(PluginException.TOO_LARGE)

        if ('/' in name || !component(name)) throw ShareContent.invalid()
    }

    /** `type/subtype` with RFC 6838 name characters, `type/` + `*`, or `*` + `/` + `*`. */
    fun validateMime(mime: String) {
        if (mime.length > MAX_MIME_BYTES) throw PluginException(PluginException.TOO_LARGE)

        val parts = mime.split('/', limit = 2)

        if (parts.size != 2) throw ShareContent.invalid()

        val (kind, subtype) = parts
        val valid =
            when {
                kind == "*" -> subtype == "*"
                subtype == "*" -> mimeName(kind)
                else -> mimeName(kind) && mimeName(subtype)
            }

        if (!valid) throw ShareContent.invalid()
    }

    fun hasControl(value: String): Boolean = value.codePoints().anyMatch { Character.getType(it) == Character.CONTROL.toInt() }

    private fun mimeName(name: String): Boolean =
        name.length in 1..MAX_MIME_NAME &&
            ascii(name[0]) &&
            name.drop(1).all { ascii(it) || it in MIME_PUNCTUATION }

    private fun ascii(character: Char) = character in 'a'..'z' || character in 'A'..'Z' || character in '0'..'9'

    /**
     * One segment: nonempty, no dot segment, no control or reserved character, no
     * trailing dot or space, no Windows device name, no reserved temporary name.
     */
    private fun component(part: String): Boolean {
        val base = part.substringBefore('.').uppercase()
        val numbered =
            base.length == DEVICE_NAME_LENGTH &&
                (base.startsWith("COM") || base.startsWith("LPT")) &&
                base.last() in '1'..'9'

        return !(
            part.isEmpty() ||
                part == "." ||
                part == ".." ||
                part.endsWith('.') ||
                part.endsWith(' ') ||
                part.startsWith(TEMP_PREFIX) ||
                base in DEVICES ||
                numbered ||
                hasControl(part) ||
                part.any { it in RESERVED }
        )
    }
}
