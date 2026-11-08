package dev.leptoskit.plugins.androidintent

import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** The Rust crate's minimal golden intent: ACTION_VIEW and nothing else. */
internal const val VIEW_ACTION = "android.intent.action.VIEW"

/**
 * The Rust crate's full golden intent (`tests/golden_full.hex`): every field
 * and every extra tag. The Rust suite checks that this file carries the same
 * hex lines.
 */
internal const val GOLDEN_FULL =
    "01000000000000141a000000616e64726f69642e696e74656e742e616374696f" +
        "6e2e53454e441d00000068747470733a2f2f6578616d706c652e636f6d2f6125" +
        "3230623f713d310a000000746578742f706c61696e0f000000636f6d2e657861" +
        "6d706c652e6170700e0000002e53686172654163746976697479020000001f00" +
        "0000616e64726f69642e696e74656e742e63617465676f72792e44454641554c" +
        "5421000000616e64726f69642e696e74656e742e63617465676f72792e42524f" +
        "575341424c450f00000004000000626f6f6c010000000100000003000000696e" +
        "7402000000f9ffffff040000006c6f6e67030000000000000000010000060000" +
        "00646f75626c6504000000000000000000044006000000737472696e67050000" +
        "000600000068c3a96c6c6f05000000656d707479050000000000000006000000" +
        "62756e646c65060000000200000005000000696e6e6572020000000100000006" +
        "000000646565706572060000000100000004000000666c616701000000000000" +
        "00050000006279746573070000000200000000ff05000000626f6f6c73080000" +
        "0002000000010000000000000004000000696e74730900000002000000010000" +
        "00ffffffff050000006c6f6e67730a000000020000000000000000000080ffff" +
        "ffffffffff7f07000000646f75626c65730b00000002000000000000000000e0" +
        "bf9c7500883ce4377e07000000737472696e67730c0000000200000001000000" +
        "6102000000ceb208000000696e745f6c6973740d00000001000000030000000b" +
        "000000737472696e675f6c6973740e000000010000000100000078"

internal fun hex(value: String): ByteArray = ByteArray(value.length / 2) { value.substring(it * 2, it * 2 + 2).toInt(HEX).toByte() }

private const val HEX = 16

/** Little-endian writer for test payloads, including ones the Rust builder refuses. */
internal class Wire {
    private val output = ByteArrayOutputStream()

    fun u32(value: Int) = apply { output.write(le(Int.SIZE_BYTES) { it.putInt(value) }) }

    fun i64(value: Long) = apply { output.write(le(Long.SIZE_BYTES) { it.putLong(value) }) }

    fun f64(value: Double) = i64(value.toRawBits())

    fun raw(bytes: ByteArray) = apply { output.write(bytes) }

    fun string(value: String) = string(value.toByteArray(Charsets.UTF_8))

    fun string(bytes: ByteArray) = u32(bytes.size).raw(bytes)

    fun bytes(): ByteArray = output.toByteArray()

    fun buffer(): ByteBuffer = ByteBuffer.wrap(bytes())

    private fun le(
        size: Int,
        write: (ByteBuffer) -> Unit,
    ): ByteArray {
        val buffer = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)
        write(buffer)

        return buffer.array()
    }
}

/** One raw bundle entry: key, tag, value bytes. */
internal data class Entry(
    val key: ByteArray,
    val tag: Int,
    val value: ByteArray,
) {
    constructor(key: String, tag: Int, value: ByteArray) : this(key.toByteArray(Charsets.UTF_8), tag, value)
}

internal fun bundleBytes(entries: List<Entry>): ByteArray {
    val wire = Wire().u32(entries.size)
    entries.forEach { wire.string(it.key).u32(it.tag).raw(it.value) }

    return wire.bytes()
}

/** Encodes an intent field by field. */
internal fun intentBytes(
    action: String? = VIEW_ACTION,
    data: String? = null,
    type: String? = null,
    packageName: String? = null,
    component: String? = null,
    categories: List<String> = emptyList(),
    flags: Int = 0,
    extras: ByteArray = bundleBytes(emptyList()),
    version: Int = 1,
): ByteArray {
    val wire = Wire().u32(version).u32(flags)
    listOf(action, data, type, packageName, component).forEach { wire.string(it ?: "") }
    wire.u32(categories.size)
    categories.forEach { wire.string(it) }

    return wire.raw(extras).bytes()
}

internal fun word(value: Int): ByteArray = Wire().u32(value).bytes()

internal fun failure(block: () -> Unit): Int {
    try {
        block()
    } catch (error: PluginException) {
        return error.code
    }

    error("expected a plugin failure")
}

internal fun expectCode(
    expected: Int,
    label: String,
    block: () -> Unit,
) {
    val code = failure(block)

    check(code == expected) { "$label: expected $expected, got $code" }
}

private fun decode(bytes: ByteArray) = IntentWire.decode(ByteBuffer.wrap(bytes))

private fun goldenChecks() {
    val minimal = decode(intentBytes())
    check(minimal == IntentSpec(0, VIEW_ACTION, null, null, null, null, emptyList(), emptyList()))

    val full = decode(hex(GOLDEN_FULL))
    check(full.action == "android.intent.action.SEND")
    check(full.flags == 0x1000_0000 or 0x0400_0000)
    check(full.data == "https://example.com/a%20b?q=1")
    check(full.type == "text/plain")
    check(full.packageName == "com.example.app")
    check(full.component == ".ShareActivity")
    check(full.categories == listOf("android.intent.category.DEFAULT", "android.intent.category.BROWSABLE"))
    val extras = full.extras.toMap()
    check(full.extras.map { it.first }.first() == "bool")
    check(extras["bool"] == ExtraValue.Bool(true))
    check(extras["int"] == ExtraValue.IntValue(-7))
    check(extras["long"] == ExtraValue.LongValue(1L shl 40))
    check(extras["double"] == ExtraValue.DoubleValue(2.5))
    check(extras["string"] == ExtraValue.Text("héllo"))
    check(extras["empty"] == ExtraValue.Text(""))
    val nested =
        ExtraValue.Nested(
            listOf(
                "inner" to ExtraValue.IntValue(1),
                "deeper" to ExtraValue.Nested(listOf("flag" to ExtraValue.Bool(false))),
            ),
        )
    check(extras["bundle"] == nested)
    check(extras["bytes"] == ExtraValue.Bytes(listOf<Byte>(0, -1)))
    check(extras["bools"] == ExtraValue.BoolArray(listOf(true, false)))
    check(extras["ints"] == ExtraValue.IntArray(listOf(1, -1)))
    check(extras["longs"] == ExtraValue.LongArray(listOf(Long.MIN_VALUE, Long.MAX_VALUE)))
    check(extras["doubles"] == ExtraValue.DoubleArray(listOf(-0.5, 1e300)))
    check(extras["strings"] == ExtraValue.StringArray(listOf("a", "β")))
    check(extras["int_list"] == ExtraValue.IntList(listOf(3)))
    check(extras["string_list"] == ExtraValue.StringList(listOf("x")))
    check(extras.size == 15)

    val bytes = hex(GOLDEN_FULL)

    for (length in bytes.indices) {
        expectCode(PluginException.INVALID_ARGUMENT, "prefix $length") { decode(bytes.copyOf(length)) }
    }

    expectCode(PluginException.INVALID_ARGUMENT, "trailing") { decode(bytes + 0.toByte()) }
}

private fun policyChecks() {
    for (flag in listOf(0x01, 0x02, 0x40, 0x80)) {
        expectCode(PluginException.PERMISSION_DENIED, "grant $flag") { decode(intentBytes(flags = flag or 0x1000_0000)) }
        // The policy is checked before anything after the flags is read.
        expectCode(PluginException.PERMISSION_DENIED, "grant first $flag") { decode(Wire().u32(1).u32(flag).bytes()) }
    }

    for (uri in listOf("file:///data/data/x/secret", "FILE:///sdcard/x", "File:x")) {
        expectCode(PluginException.PERMISSION_DENIED, uri) { decode(intentBytes(data = uri)) }
    }

    for (flag in listOf(0x04, 0x08, 0x10, 0x20, 0x1000_0000, 0x4000_0000, Int.MIN_VALUE)) {
        decode(intentBytes(flags = flag))
    }
}

private fun fieldChecks() {
    val valid =
        listOf(
            intentBytes(data = "content://com.example.provider/item/1"),
            intentBytes(data = "package:com.example"),
            intentBytes(data = "x-custom+v1.2:rest"),
            intentBytes(data = "https://a/%41%7e"),
            intentBytes(type = "*/*"),
            intentBytes(type = "application/vnd.android.package-archive"),
            intentBytes(packageName = "android"),
            intentBytes(packageName = "com.Example_1.app2", component = "a.b.Outer\$Inner"),
            intentBytes(action = null, packageName = "com.example", component = ".Main"),
            intentBytes(packageName = "com.example", component = "_x.\$y9"),
        )
    valid.forEach { decode(it) }

    val invalid =
        listOf(
            intentBytes(data = "no-scheme"),
            intentBytes(data = ":missing"),
            intentBytes(data = "1http://a"),
            intentBytes(data = "https:"),
            intentBytes(data = "https://a b"),
            intentBytes(data = "https://a\\b"),
            intentBytes(data = "https://é"),
            intentBytes(data = "https://a/%"),
            intentBytes(data = "https://a/%4"),
            intentBytes(data = "https://a/%zz"),
            intentBytes(data = "https://a/%00"),
            intentBytes(data = "https://a/%1F"),
            intentBytes(data = "https://a/%7f"),
            intentBytes(type = "text"),
            intentBytes(type = "Text/plain"),
            intentBytes(type = "text/plain; charset=utf-8"),
            intentBytes(type = "a/b/c"),
            intentBytes(packageName = "com..app"),
            intentBytes(packageName = "com.1app"),
            intentBytes(packageName = "_com"),
            intentBytes(packageName = "com.example", component = "..Main"),
            intentBytes(packageName = "com.example", component = "com.9Main"),
            intentBytes(packageName = "com.example", component = "."),
            intentBytes(action = "a b"),
            intentBytes(action = "é"),
            intentBytes(categories = listOf("")),
            intentBytes(categories = listOf("a", "a")),
            intentBytes(action = null),
            intentBytes(action = null, packageName = "com.example"),
            intentBytes(component = "com.example.Main"),
            intentBytes(version = 0),
            intentBytes(version = 2),
            intentBytes(version = -1),
        )
    invalid.forEachIndexed { index, bytes ->
        expectCode(PluginException.INVALID_ARGUMENT, "invalid field $index") { decode(bytes) }
    }

    expectCode(PluginException.INVALID_ARGUMENT, "bad utf-8 action") {
        decode(
            Wire()
                .u32(1)
                .u32(0)
                .string(byteArrayOf(-1))
                .bytes(),
        )
    }
}

private fun boundChecks() {
    val name = { length: Int -> "a".repeat(length) }
    decode(intentBytes(action = name(IntentWire.MAX_NAME_BYTES)))
    expectCode(PluginException.TOO_LARGE, "action") { decode(intentBytes(action = name(IntentWire.MAX_NAME_BYTES + 1))) }
    expectCode(PluginException.TOO_LARGE, "package") { decode(intentBytes(packageName = name(IntentWire.MAX_NAME_BYTES + 1))) }
    expectCode(PluginException.TOO_LARGE, "category") { decode(intentBytes(categories = listOf(name(IntentWire.MAX_NAME_BYTES + 1)))) }
    decode(intentBytes(type = "a/" + "b".repeat(IntentWire.MAX_MIME_BYTES - 2)))
    expectCode(PluginException.TOO_LARGE, "mime") { decode(intentBytes(type = "a/" + "b".repeat(IntentWire.MAX_MIME_BYTES - 1))) }
    decode(intentBytes(data = "https://a/" + "b".repeat(IntentWire.MAX_URI_BYTES - 10)))
    expectCode(PluginException.TOO_LARGE, "uri") { decode(intentBytes(data = "https://a/" + "b".repeat(IntentWire.MAX_URI_BYTES - 9))) }
    val categories = List(IntentWire.MAX_CATEGORIES) { "c.$it" }
    decode(intentBytes(categories = categories))
    expectCode(PluginException.TOO_LARGE, "categories") { decode(intentBytes(categories = categories + "c.extra")) }

    // A declared length past its bound is too large even when truncated.
    expectCode(PluginException.TOO_LARGE, "truncated oversize") {
        decode(
            Wire()
                .u32(1)
                .u32(0)
                .u32(IntentWire.MAX_NAME_BYTES + 1)
                .bytes(),
        )
    }

    expectCode(PluginException.TOO_LARGE, "payload") { decode(ByteArray(IntentWire.MAX_INTENT_BYTES + 1)) }
    expectCode(PluginException.INVALID_ARGUMENT, "payload at bound") { decode(ByteArray(IntentWire.MAX_INTENT_BYTES)) }

    // 62 bytes of minimal intent, 14 bytes of framing per string extra.
    val first = Wire().string("a".repeat(IntentWire.MAX_VALUE_BYTES)).bytes()
    val fill = IntentWire.MAX_INTENT_BYTES - 62 - 2 * 14 - IntentWire.MAX_VALUE_BYTES
    val exact = { size: Int ->
        intentBytes(extras = bundleBytes(listOf(Entry("k0", 5, first), Entry("k1", 5, Wire().string("b".repeat(size)).bytes()))))
    }
    check(exact(fill).size == IntentWire.MAX_INTENT_BYTES)
    decode(exact(fill))
    expectCode(PluginException.TOO_LARGE, "whole intent") { decode(exact(fill + 1)) }
}

private fun extraBoundChecks() {
    val withExtras = { entries: List<Entry> -> intentBytes(extras = bundleBytes(entries)) }
    val wide = { count: Int -> bundleBytes(List(count) { Entry("k$it", 2, word(it)) }) }
    decode(intentBytes(extras = wide(IntentWire.MAX_EXTRAS)))
    expectCode(PluginException.TOO_LARGE, "wide") { decode(intentBytes(extras = wide(IntentWire.MAX_EXTRAS + 1))) }

    // The intent's own extras are depth 1.
    val nested = { depth: Int ->
        (1 until depth).fold(bundleBytes(listOf(Entry("leaf", 2, word(1))))) { inner, _ -> bundleBytes(listOf(Entry("n", 6, inner))) }
    }
    decode(intentBytes(extras = nested(IntentWire.MAX_DEPTH)))
    expectCode(PluginException.TOO_LARGE, "deep") { decode(intentBytes(extras = nested(IntentWire.MAX_DEPTH + 1))) }

    val total = { last: Int ->
        bundleBytes(List(4) { Entry("b$it", 6, wide(if (it == 3) last else IntentWire.MAX_EXTRAS - 1)) })
    }
    decode(intentBytes(extras = total(IntentWire.MAX_EXTRAS - 1)))
    expectCode(PluginException.TOO_LARGE, "total") { decode(intentBytes(extras = total(IntentWire.MAX_EXTRAS))) }

    val longKey = "k".repeat(IntentWire.MAX_NAME_BYTES + 1)
    expectCode(PluginException.TOO_LARGE, "key") { decode(withExtras(listOf(Entry(longKey, 2, word(0))))) }
    val longValue = word(IntentWire.MAX_VALUE_BYTES + 1)
    expectCode(PluginException.TOO_LARGE, "string") { decode(withExtras(listOf(Entry("s", 5, longValue)))) }
    expectCode(PluginException.TOO_LARGE, "bytes") { decode(withExtras(listOf(Entry("s", 7, longValue)))) }

    for (tag in 8..14) {
        // Longs and doubles are eight bytes; every other element is a zero word.
        val element = if (tag == 10 || tag == 11) ByteArray(Long.SIZE_BYTES) else word(0)
        val full = Wire().u32(IntentWire.MAX_ARRAY_LEN)
        repeat(IntentWire.MAX_ARRAY_LEN) { full.raw(element) }
        decode(withExtras(listOf(Entry("a", tag, full.bytes()))))
        expectCode(PluginException.TOO_LARGE, "array $tag") {
            decode(withExtras(listOf(Entry("a", tag, word(IntentWire.MAX_ARRAY_LEN + 1)))))
        }
    }
}

private fun malformedExtraChecks() {
    val withExtras = { entries: List<Entry> -> intentBytes(extras = bundleBytes(entries)) }
    val nan = Wire().f64(Double.NaN).bytes()
    val cases =
        listOf(
            "unknown tag" to listOf(Entry("k", 15, byteArrayOf())),
            "tag zero" to listOf(Entry("k", 0, byteArrayOf())),
            "bool word 2" to listOf(Entry("k", 1, word(2))),
            "bool array word 2" to listOf(Entry("k", 8, word(1) + word(2))),
            "NaN" to listOf(Entry("k", 4, nan)),
            "infinity" to listOf(Entry("k", 4, Wire().f64(Double.POSITIVE_INFINITY).bytes())),
            "NaN element" to listOf(Entry("k", 11, word(1) + nan)),
            "empty key" to listOf(Entry("", 2, word(0))),
            "control key" to listOf(Entry("a\nb", 2, word(0))),
            "C1 control key" to listOf(Entry("a\u0085", 2, word(0))),
            "invalid key" to listOf(Entry(byteArrayOf(-1), 2, word(0))),
            "duplicate key" to listOf(Entry("k", 2, word(0)), Entry("k", 2, word(0))),
            "invalid string" to listOf(Entry("k", 5, word(1) + byteArrayOf(-1))),
        )

    for ((label, entries) in cases) {
        expectCode(PluginException.INVALID_ARGUMENT, label) { decode(withExtras(entries)) }
    }

    // The same key in different bundles is fine.
    decode(withExtras(listOf(Entry("k", 6, bundleBytes(listOf(Entry("k", 2, word(0))))))))
}

private fun chooserAndUriChecks() {
    val chooser = IntentWire.decodeChooser(ByteBuffer.wrap(Wire().string("Go").raw(intentBytes()).bytes()))
    check(chooser.title == "Go" && chooser.intent.action == VIEW_ACTION)
    check(IntentWire.decodeChooser(ByteBuffer.wrap(Wire().string("").raw(intentBytes()).bytes())).title.isEmpty())
    IntentWire.decodeChooser(ByteBuffer.wrap(Wire().string("t".repeat(IntentWire.MAX_TITLE_BYTES)).raw(intentBytes()).bytes()))
    expectCode(PluginException.TOO_LARGE, "title") {
        IntentWire.decodeChooser(ByteBuffer.wrap(Wire().string("t".repeat(IntentWire.MAX_TITLE_BYTES + 1)).raw(intentBytes()).bytes()))
    }
    expectCode(PluginException.INVALID_ARGUMENT, "title control") {
        IntentWire.decodeChooser(ByteBuffer.wrap(Wire().string("a\nb").raw(intentBytes()).bytes()))
    }
    expectCode(PluginException.INVALID_ARGUMENT, "chooser without intent") { IntentWire.decodeChooser(Wire().string("t").buffer()) }

    val uri = { value: String -> IntentWire.decodeIntentUri(ByteBuffer.wrap(value.toByteArray(Charsets.UTF_8))) }
    check(uri("intent:#Intent;action=android.settings.SETTINGS;end") == "intent:#Intent;action=android.settings.SETTINGS;end")
    uri("intent://scan/#Intent;scheme=zxing;package=com.google.zxing.client.android;end")

    for (value in listOf(
        "",
        "https://example.com",
        "INTENT:#Intent;end",
        "intent://a",
        "intent://a#Intent;",
        "intent://a/%00#Intent;end",
    )) {
        expectCode(PluginException.INVALID_ARGUMENT, value) { uri(value) }
    }

    val exact = "intent://" + "a".repeat(IntentWire.MAX_URI_BYTES - 20) + "#Intent;end"
    check(uri(exact).length == IntentWire.MAX_URI_BYTES)
    expectCode(PluginException.TOO_LARGE, "long uri") { uri("intent://" + "a".repeat(IntentWire.MAX_URI_BYTES) + "#Intent;end") }
    expectCode(PluginException.INVALID_ARGUMENT, "uri utf-8") { IntentWire.decodeIntentUri(ByteBuffer.wrap(byteArrayOf(-1))) }

    val explicit = decode(intentBytes(packageName = "com.example"))
    check(IntentWire.requireExplicit(explicit) == explicit)
    expectCode(PluginException.INVALID_ARGUMENT, "implicit service") { IntentWire.requireExplicit(decode(intentBytes())) }
}

private fun replyChecks() {
    check(IntentWire.booleanReply(true).contentEquals(byteArrayOf(1, 0, 0, 0)))
    check(IntentWire.booleanReply(false).contentEquals(byteArrayOf(0, 0, 0, 0)))
    check(IntentWire.unresolvedReply().contentEquals(byteArrayOf(0, 0, 0, 0)))

    // The same bytes the Rust suite expects for this reply.
    val settings = IntentWire.resolvedReply("com.android.settings", "com.android.settings.Settings", "Settings")
    val expected =
        Wire()
            .u32(1)
            .string("com.android.settings")
            .string("com.android.settings.Settings")
            .string("Settings")
            .bytes()
    check(settings.contentEquals(expected))

    // Labels are cut at a character boundary, never inside one.
    val label = "a" + "é".repeat(IntentWire.MAX_LABEL_BYTES)
    val reply = ByteBuffer.wrap(IntentWire.resolvedReply("android", "a.B", label)).order(ByteOrder.LITTLE_ENDIAN)
    reply.position(4 + 4 + "android".length + 4 + "a.B".length)
    val size = reply.int
    check(size == IntentWire.MAX_LABEL_BYTES - 1) { "label size $size" }
    check(IntentWire.truncate("ab", 1).contentEquals(byteArrayOf('a'.code.toByte())))
    check(IntentWire.truncate("é", 1).isEmpty())

    for ((packageName, activity) in listOf("com..x" to "a.B", "com.x" to ".B", "com.x" to "a..B", "a".repeat(513) to "a.B")) {
        expectCode(PluginException.NATIVE, "$packageName/$activity") { IntentWire.resolvedReply(packageName, activity, "") }
    }
}

/** Every pure decoding rule, runnable on a plain JVM and inside the instrumented suite. */
fun androidIntentWireChecks() {
    goldenChecks()
    policyChecks()
    fieldChecks()
    boundChecks()
    extraBoundChecks()
    malformedExtraChecks()
    chooserAndUriChecks()
    replyChecks()
}
