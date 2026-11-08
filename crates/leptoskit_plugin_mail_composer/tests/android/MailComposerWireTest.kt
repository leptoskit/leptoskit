package dev.leptoskit.plugins.mailcomposer

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

private fun wire(
    to: List<String>,
    cc: List<String>,
    bcc: List<String>,
    subject: String,
    body: String,
): ByteBuffer {
    fun strings(values: List<String>) = values.map { it.toByteArray(Charsets.UTF_8) }
    val toBytes = strings(to)
    val ccBytes = strings(cc)
    val bccBytes = strings(bcc)
    val subjectBytes = subject.toByteArray(Charsets.UTF_8)
    val bodyBytes = body.toByteArray(Charsets.UTF_8)
    val size =
        listOf(toBytes, ccBytes, bccBytes).sumOf { list -> 4 + list.sumOf { 4 + it.size } } +
            4 + subjectBytes.size + 4 + bodyBytes.size
    val result = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)

    fun putList(values: List<ByteArray>) {
        result.putInt(values.size)
        for (bytes in values) result.putInt(bytes.size).put(bytes)
    }
    putList(toBytes)
    putList(ccBytes)
    putList(bccBytes)
    result.putInt(subjectBytes.size).put(subjectBytes)
    result.putInt(bodyBytes.size).put(bodyBytes)
    result.flip()

    return result
}

/** Exercises mail compose framing without opening a mail client. */
fun main() {
    val decoded =
        MailCompose.decode(
            wire(listOf("a@example.test"), listOf("c@example.test"), emptyList(), "Hi", "Body"),
        )
    check(decoded.to == listOf("a@example.test"))
    check(decoded.cc == listOf("c@example.test"))
    check(decoded.bcc.isEmpty())
    check(decoded.subject == "Hi")
    check(decoded.body == "Body")

    for (bad in listOf("", "no-at", "@domain", "a b@c")) {
        try {
            MailCompose.decode(wire(listOf(bad), emptyList(), emptyList(), "", ""))
            error("Expected rejection: $bad")
        } catch (error: PluginException) {
            check(error.code == PluginException.INVALID_ARGUMENT)
        }
    }

    println("Mail composer synthetic wire and bounds checks passed")
}
