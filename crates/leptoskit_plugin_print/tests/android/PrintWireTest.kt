package dev.leptoskit.plugins.print

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

private fun wire(kind: Int, body: String): ByteBuffer {
    val bytes = body.toByteArray(Charsets.UTF_8)
    val result = ByteBuffer.allocate(8 + bytes.size).order(ByteOrder.LITTLE_ENDIAN)
    result.putInt(kind).putInt(bytes.size).put(bytes)
    result.flip()

    return result
}

/** Exercises print job framing without opening PrintManager. */
fun main() {
    check(PrintJob.decode(wire(0, "<p>hi</p>")) is PrintJob.Html)
    check((PrintJob.decode(wire(1, "https://example.test/a.pdf")) as PrintJob.PdfUri).uri == "https://example.test/a.pdf")
    check((PrintJob.decode(wire(1, "file:///tmp/a.pdf")) as PrintJob.PdfUri).uri == "file:///tmp/a.pdf")

    for (bad in listOf(
        "mailto:a@example.test",
        "javascript:alert(1)",
        "https://user:pass@example.test/a.pdf",
        "https://example.test/a b.pdf",
        "file://host/path.pdf",
    )) {
        try {
            PrintJob.decode(wire(1, bad))
            error("Expected rejection: $bad")
        } catch (error: PluginException) {
            check(error.code == PluginException.INVALID_ARGUMENT || error.code == PluginException.TOO_LARGE)
        }
    }

    try {
        PrintJob.decode(wire(9, "x"))
        error("Expected unknown kind rejection")
    } catch (error: PluginException) {
        check(error.code == PluginException.INVALID_ARGUMENT)
    }

    println("Print synthetic wire and bounds checks passed")
}
