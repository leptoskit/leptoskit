package dev.leptoskit.plugins.androidintent

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

/** One extra, as android_intent_plus's `arguments` and `arrayArguments` allow. No parcelables. */
internal sealed interface ExtraValue {
    data class Bool(
        val value: Boolean,
    ) : ExtraValue

    data class IntValue(
        val value: Int,
    ) : ExtraValue

    data class LongValue(
        val value: Long,
    ) : ExtraValue

    data class DoubleValue(
        val value: Double,
    ) : ExtraValue

    data class Text(
        val value: String,
    ) : ExtraValue

    data class Nested(
        val entries: List<Pair<String, ExtraValue>>,
    ) : ExtraValue

    data class Bytes(
        val value: List<Byte>,
    ) : ExtraValue

    data class BoolArray(
        val value: List<Boolean>,
    ) : ExtraValue

    data class IntArray(
        val value: List<Int>,
    ) : ExtraValue

    data class LongArray(
        val value: List<Long>,
    ) : ExtraValue

    data class DoubleArray(
        val value: List<Double>,
    ) : ExtraValue

    data class StringArray(
        val value: List<String>,
    ) : ExtraValue

    data class IntList(
        val value: List<Int>,
    ) : ExtraValue

    data class StringList(
        val value: List<String>,
    ) : ExtraValue
}

/** A decoded, validated intent; building the Android `Intent` happens in the plugin. */
internal data class IntentSpec(
    val flags: Int,
    val action: String?,
    val data: String?,
    val type: String?,
    val packageName: String?,
    val component: String?,
    val categories: List<String>,
    val extras: List<Pair<String, ExtraValue>>,
) {
    val isExplicit: Boolean get() = packageName != null || component != null
}

internal data class ChooserSpec(
    val title: String,
    val intent: IntentSpec,
)

/**
 * The trust boundary for guest requests. Applies the Rust crate's rules in the
 * same order, because a guest can send any bytes it likes: TOO_LARGE past a
 * bound, PERMISSION_DENIED for URI grants and `file:` URIs, INVALID_ARGUMENT
 * for anything malformed.
 */
internal object IntentWire {
    const val WIRE_VERSION = 1L
    const val MAX_INTENT_BYTES = 128 * 1024
    const val MAX_NAME_BYTES = 512
    const val MAX_MIME_BYTES = 255
    const val MAX_URI_BYTES = 8192
    const val MAX_VALUE_BYTES = 64 * 1024
    const val MAX_ARRAY_LEN = 1024
    const val MAX_EXTRAS = 64
    const val MAX_TOTAL_EXTRAS = 256
    const val MAX_DEPTH = 4
    const val MAX_CATEGORIES = 8
    const val MAX_TITLE_BYTES = 1024
    const val MAX_LABEL_BYTES = 1024

    /** FLAG_GRANT_READ, _WRITE, _PERSISTABLE and _PREFIX_URI_PERMISSION. */
    const val GRANT_URI_PERMISSION_FLAGS = 0x01 or 0x02 or 0x40 or 0x80

    private const val TAG_BOOL = 1
    private const val TAG_INT = 2
    private const val TAG_LONG = 3
    private const val TAG_DOUBLE = 4
    private const val TAG_STRING = 5
    private const val TAG_BUNDLE = 6
    private const val TAG_BYTES = 7
    private const val TAG_BOOL_ARRAY = 8
    private const val TAG_INT_ARRAY = 9
    private const val TAG_LONG_ARRAY = 10
    private const val TAG_DOUBLE_ARRAY = 11
    private const val TAG_STRING_ARRAY = 12
    private const val TAG_INT_LIST = 13
    private const val TAG_STRING_LIST = 14

    fun decode(payload: ByteBuffer): IntentSpec {
        if (payload.remaining() > MAX_INTENT_BYTES) throw tooLarge()

        val input = WireReader(payload)

        if (input.u32() != WIRE_VERSION) throw invalid()

        val flags = input.u32().toInt()
        requireFlags(flags)
        val action = input.optional(MAX_NAME_BYTES)?.also(::requireName)
        val data = input.optional(MAX_URI_BYTES)?.also(::requireData)
        val type = input.optional(MAX_MIME_BYTES)?.also(::requireMime)
        val packageName = input.optional(MAX_NAME_BYTES)?.also(::requirePackage)
        val component = input.optional(MAX_NAME_BYTES)?.also(::requireClass)
        val categories = categories(input)
        val extras = bundle(input, 1, Counter())
        input.finish()

        if ((component != null && packageName == null) || (action == null && component == null)) throw invalid()

        return IntentSpec(flags, action, data, type, packageName, component, categories, extras)
    }

    fun decodeChooser(payload: ByteBuffer): ChooserSpec {
        val input = WireReader(payload)
        val title = input.string(MAX_TITLE_BYTES)
        requirePlainText(title)

        return ChooserSpec(title, decode(input.rest()))
    }

    fun decodeIntentUri(payload: ByteBuffer): String {
        if (payload.remaining() > MAX_URI_BYTES) throw tooLarge()

        val value = utf8(payload.duplicate())
        requireIntentUri(value)

        return value
    }

    /** Android refuses implicit service intents. */
    fun requireExplicit(spec: IntentSpec): IntentSpec {
        if (!spec.isExplicit) throw invalid()

        return spec
    }

    fun booleanReply(value: Boolean): ByteArray = byteArrayOf(if (value) 1 else 0, 0, 0, 0)

    /** Encodes a resolved activity; a value outside the contract is a native failure, never a guess. */
    fun resolvedReply(
        packageName: String,
        activityName: String,
        label: String,
    ): ByteArray {
        val names = listOf(packageName, activityName).map { it.toByteArray(Charsets.UTF_8) }

        try {
            requirePackage(packageName)
            requireClass(activityName)
        } catch (error: PluginException) {
            throw PluginException(PluginException.NATIVE, error)
        }

        if (names.any { it.size > MAX_NAME_BYTES } || activityName.startsWith(".")) {
            throw PluginException(PluginException.NATIVE)
        }

        val text = truncate(label, MAX_LABEL_BYTES)
        val output = ByteBuffer.allocate(4 + names.sumOf { 4 + it.size } + 4 + text.size).order(ByteOrder.LITTLE_ENDIAN)
        output.putInt(1)
        names.forEach { output.putInt(it.size).put(it) }
        output.putInt(text.size).put(text)

        return output.array()
    }

    /** The empty reply for "no activity". */
    fun unresolvedReply(): ByteArray = byteArrayOf(0, 0, 0, 0)

    /** UTF-8 bytes of at most [max], cut at a character boundary. */
    fun truncate(
        value: String,
        max: Int,
    ): ByteArray {
        val bytes = value.toByteArray(Charsets.UTF_8)

        if (bytes.size <= max) return bytes

        var end = max

        while (end > 0 && (bytes[end].toInt() and CONTINUATION_MASK) == CONTINUATION_BITS) end--

        return bytes.copyOf(end)
    }

    private fun categories(input: WireReader): List<String> {
        val count = input.count(MAX_CATEGORIES)
        val categories = ArrayList<String>(count)

        repeat(count) {
            val category = input.string(MAX_NAME_BYTES)
            requireName(category)

            if (category in categories) throw invalid()

            categories.add(category)
        }

        return categories
    }

    private class Counter(
        var total: Int = 0,
    )

    private fun bundle(
        input: WireReader,
        depth: Int,
        counter: Counter,
    ): List<Pair<String, ExtraValue>> {
        val count = input.count(MAX_EXTRAS)
        val entries = ArrayList<Pair<String, ExtraValue>>(count)
        val keys = HashSet<String>()

        repeat(count) {
            counter.total += 1

            if (counter.total > MAX_TOTAL_EXTRAS) throw tooLarge()

            val key = input.string(MAX_NAME_BYTES)
            requireKey(key)

            if (!keys.add(key)) throw invalid()

            entries.add(key to value(input, depth, counter))
        }

        return entries
    }

    private fun value(
        input: WireReader,
        depth: Int,
        counter: Counter,
    ): ExtraValue =
        when (input.u32().toInt()) {
            TAG_BOOL -> ExtraValue.Bool(input.bool())
            TAG_INT -> ExtraValue.IntValue(input.i32())
            TAG_LONG -> ExtraValue.LongValue(input.i64())
            TAG_DOUBLE -> ExtraValue.DoubleValue(input.f64())
            TAG_STRING -> ExtraValue.Text(input.string(MAX_VALUE_BYTES))
            TAG_BUNDLE -> nested(input, depth, counter)
            TAG_BYTES -> ExtraValue.Bytes(input.bytes(MAX_VALUE_BYTES).toList())
            TAG_BOOL_ARRAY -> ExtraValue.BoolArray(input.list { it.bool() })
            TAG_INT_ARRAY -> ExtraValue.IntArray(input.list { it.i32() })
            TAG_LONG_ARRAY -> ExtraValue.LongArray(input.list { it.i64() })
            TAG_DOUBLE_ARRAY -> ExtraValue.DoubleArray(input.list { it.f64() })
            TAG_STRING_ARRAY -> ExtraValue.StringArray(input.list { it.string(MAX_VALUE_BYTES) })
            TAG_INT_LIST -> ExtraValue.IntList(input.list { it.i32() })
            TAG_STRING_LIST -> ExtraValue.StringList(input.list { it.string(MAX_VALUE_BYTES) })
            else -> throw invalid()
        }

    private fun nested(
        input: WireReader,
        depth: Int,
        counter: Counter,
    ): ExtraValue {
        if (depth >= MAX_DEPTH) throw tooLarge()

        return ExtraValue.Nested(bundle(input, depth + 1, counter))
    }

    fun requireFlags(flags: Int) {
        if (flags and GRANT_URI_PERMISSION_FLAGS != 0) throw PluginException(PluginException.PERMISSION_DENIED)
    }

    /** Printable ASCII with no whitespace. */
    fun requireName(value: String) {
        if (value.isEmpty() || !value.all { it.code in FIRST_GRAPHIC..LAST_GRAPHIC }) throw invalid()
    }

    fun requirePackage(value: String) {
        val valid =
            value.isNotEmpty() &&
                value.split('.').all { segment ->
                    segment.isNotEmpty() &&
                        segment[0].isAsciiLetter() &&
                        segment.all { it.isAsciiLetter() || it.isAsciiDigit() || it == '_' }
                }

        if (!valid) throw invalid()
    }

    /** A Java class name; a leading `.` is relative to the package. */
    fun requireClass(value: String) {
        val absolute = value.removePrefix(".")
        val identifier = { char: Char -> char.isAsciiLetter() || char == '_' || char == '$' }
        val valid =
            absolute.isNotEmpty() &&
                absolute.split('.').all { segment ->
                    segment.isNotEmpty() && identifier(segment[0]) && segment.all { identifier(it) || it.isAsciiDigit() }
                }

        if (!valid) throw invalid()
    }

    /** Lowercase `type/subtype`, because Android matches MIME types case-sensitively. */
    fun requireMime(value: String) {
        val slash = value.indexOf('/')

        if (slash < 0 || !mimeToken(value.substring(0, slash)) || !mimeToken(value.substring(slash + 1))) throw invalid()
    }

    private fun mimeToken(token: String): Boolean =
        token.isNotEmpty() && token.all { it in 'a'..'z' || it.isAsciiDigit() || it in MIME_SYMBOLS }

    /** An absolute URI; `file:` URIs are refused as a policy, not a syntax error. */
    fun requireData(value: String) {
        requireEncoded(value)
        val colon = value.indexOf(':')

        if (colon < 0) throw invalid()

        val scheme = value.substring(0, colon)
        val schemeValid =
            scheme.isNotEmpty() &&
                scheme[0].isAsciiLetter() &&
                scheme.all { it.isAsciiLetter() || it.isAsciiDigit() || it == '+' || it == '-' || it == '.' }

        if (!schemeValid || colon == value.length - 1) throw invalid()

        if (scheme.equals("file", ignoreCase = true)) throw PluginException(PluginException.PERMISSION_DENIED)
    }

    /** Printable ASCII, no backslash, and escapes that never decode to a control character. */
    fun requireEncoded(value: String) {
        if (value.isEmpty() || !value.all { it.code in FIRST_GRAPHIC..LAST_GRAPHIC && it != '\\' }) throw invalid()

        value.forEachIndexed { index, char -> if (char == '%') requireEscape(value, index) }
    }

    private fun requireEscape(
        value: String,
        index: Int,
    ) {
        if (index + ESCAPE_LENGTH > value.length) throw invalid()

        val digits = value.substring(index + 1, index + 3)

        if (!digits.all { it.isAsciiHexDigit() } || digits[0] == '0' || digits[0] == '1' || digits.equals("7f", ignoreCase = true)) {
            throw invalid()
        }
    }

    fun requireIntentUri(value: String) {
        requireEncoded(value)

        if (!value.startsWith("intent:") || !value.contains("#Intent;") || !value.endsWith(";end")) throw invalid()
    }

    fun requireKey(value: String) {
        if (value.isEmpty()) throw invalid()

        requirePlainText(value)
    }

    fun requirePlainText(value: String) {
        if (value.any { it.isISOControl() }) throw invalid()
    }

    private fun Char.isAsciiLetter() = this in 'a'..'z' || this in 'A'..'Z'

    private fun Char.isAsciiDigit() = this in '0'..'9'

    private fun Char.isAsciiHexDigit() = isAsciiDigit() || this in 'a'..'f' || this in 'A'..'F'

    private const val ESCAPE_LENGTH = 3
    private const val FIRST_GRAPHIC = 0x21
    private const val LAST_GRAPHIC = 0x7e
    private const val MIME_SYMBOLS = "!#$&^_.+-*"
    private const val CONTINUATION_MASK = 0xc0
    private const val CONTINUATION_BITS = 0x80

    internal fun invalid() = PluginException(PluginException.INVALID_ARGUMENT)

    internal fun tooLarge() = PluginException(PluginException.TOO_LARGE)

    internal fun utf8(buffer: ByteBuffer): String =
        try {
            Charsets.UTF_8
                .newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(buffer)
                .toString()
        } catch (_: CharacterCodingException) {
            throw invalid()
        }
}

/** Little-endian reads that check each bound before the bytes it guards. */
internal class WireReader(
    payload: ByteBuffer,
) {
    private val buffer = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

    private fun need(size: Int) {
        if (buffer.remaining() < size) throw IntentWire.invalid()
    }

    /** An unsigned 32-bit word, widened so counts compare correctly. */
    fun u32(): Long {
        need(Int.SIZE_BYTES)

        return buffer.int.toLong() and UNSIGNED_MASK
    }

    fun i32(): Int {
        need(Int.SIZE_BYTES)

        return buffer.int
    }

    fun i64(): Long {
        need(Long.SIZE_BYTES)

        return buffer.long
    }

    fun f64(): Double {
        need(Long.SIZE_BYTES)
        val value = Double.fromBits(buffer.long)

        if (!value.isFinite()) throw IntentWire.invalid()

        return value
    }

    fun bool(): Boolean =
        when (u32()) {
            0L -> false
            1L -> true
            else -> throw IntentWire.invalid()
        }

    fun count(max: Int): Int {
        val value = u32()

        if (value > max) throw IntentWire.tooLarge()

        return value.toInt()
    }

    fun bytes(max: Int): ByteArray {
        val size = count(max)
        need(size)
        val bytes = ByteArray(size)
        buffer.get(bytes)

        return bytes
    }

    fun string(max: Int): String = IntentWire.utf8(ByteBuffer.wrap(bytes(max)))

    /** The empty string means the field is absent. */
    fun optional(max: Int): String? = string(max).ifEmpty { null }

    fun <T> list(read: (WireReader) -> T): List<T> {
        val size = count(IntentWire.MAX_ARRAY_LEN)

        return List(size) { read(this) }
    }

    fun finish() {
        if (buffer.hasRemaining()) throw IntentWire.invalid()
    }

    /** Reads continue from where this reader stopped. */
    fun rest(): ByteBuffer = buffer.slice().order(ByteOrder.LITTLE_ENDIAN)

    private companion object {
        const val UNSIGNED_MASK = 0xffff_ffffL
    }
}
