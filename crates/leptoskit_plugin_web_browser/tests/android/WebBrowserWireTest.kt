package dev.leptoskit.plugins.webbrowser

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

private fun wire(url: String): ByteBuffer {
    val bytes = url.toByteArray(Charsets.UTF_8)
    val result = ByteBuffer.allocate(4 + bytes.size).order(ByteOrder.LITTLE_ENDIAN)
    result.putInt(bytes.size).put(bytes)
    result.flip()

    return result
}

/** Exercises browser URL framing without opening Custom Tabs. */
fun main() {
    check(WebBrowserOpen.decode(wire("https://example.test/a%20b")).url == "https://example.test/a%20b")

    for (bad in listOf(
        "file:///secret",
        "mailto:a@example.test",
        "https://user:pass@example.test",
        "https://example.test/a b",
    )) {
        try {
            WebBrowserOpen.decode(wire(bad))
            error("Expected rejection: $bad")
        } catch (error: PluginException) {
            check(error.code == PluginException.INVALID_ARGUMENT)
        }
    }

    println("Web browser synthetic wire and bounds checks passed")
}
