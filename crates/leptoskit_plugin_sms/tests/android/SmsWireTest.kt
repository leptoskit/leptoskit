package dev.leptoskit.plugins.sms

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

private fun wire(
    recipients: List<String>,
    body: String,
): ByteBuffer {
    val recipientBytes = recipients.map { it.toByteArray(Charsets.UTF_8) }
    val bodyBytes = body.toByteArray(Charsets.UTF_8)
    val size = 4 + recipientBytes.sumOf { 4 + it.size } + 4 + bodyBytes.size
    val result = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)
    result.putInt(recipients.size)
    for (bytes in recipientBytes) {
        result.putInt(bytes.size).put(bytes)
    }
    result.putInt(bodyBytes.size).put(bodyBytes)
    result.flip()

    return result
}

/** Exercises SMS compose framing without opening a messenger. */
fun main() {
    val decoded = SmsCompose.decode(wire(listOf("+1 555 0100"), "hello"))
    check(decoded.recipients == listOf("+1 555 0100"))
    check(decoded.body == "hello")
    check(SmsCompose.decode(wire(emptyList(), "")).body.isEmpty())

    for (bad in listOf("", "abc", "555\u0000")) {
        try {
            SmsCompose.decode(wire(listOf(bad), ""))
            error("Expected rejection: $bad")
        } catch (error: PluginException) {
            check(error.code == PluginException.INVALID_ARGUMENT)
        }
    }

    try {
        SmsCompose.decode(wire(List(33) { "1" }, ""))
        error("Expected recipient bound failure")
    } catch (error: PluginException) {
        check(error.code == PluginException.TOO_LARGE)
    }

    println("SMS synthetic wire and bounds checks passed")
}
