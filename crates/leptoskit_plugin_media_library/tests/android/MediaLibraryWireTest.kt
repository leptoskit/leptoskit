package dev.leptoskit.plugins.medialibrary

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

private fun listWire(
    kind: Int,
    limit: Int,
): ByteBuffer {
    val result = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN)
    result.putInt(kind).putInt(limit)
    result.flip()
    return result
}

private fun saveWire(
    kind: Int,
    uri: String,
): ByteBuffer {
    val bytes = uri.toByteArray(Charsets.UTF_8)
    val result = ByteBuffer.allocate(8 + bytes.size).order(ByteOrder.LITTLE_ENDIAN)
    result.putInt(kind).putInt(bytes.size).put(bytes)
    result.flip()
    return result
}

/** Exercises media-library framing without touching MediaStore. */
fun main() {
    check(ListOptions.decode(listWire(2, 8)).limit == 8)
    check(SaveRequest.decode(saveWire(0, "file:///tmp/a.jpg")).uri == "file:///tmp/a.jpg")

    try {
        ListOptions.decode(listWire(0, 0))
        error("expected zero limit rejection")
    } catch (error: PluginException) {
        check(error.code == PluginException.INVALID_ARGUMENT)
    }

    try {
        SaveRequest.decode(saveWire(2, "file:///tmp/a.jpg"))
        error("expected All kind rejection")
    } catch (error: PluginException) {
        check(error.code == PluginException.INVALID_ARGUMENT)
    }

    val asset =
        Asset(
            id = "1",
            kind = MediaKind.Images,
            uri = "content://media/1",
            width = 10,
            height = 20,
            creationTimeMs = 1000L,
        )
    check(encodeAssets(listOf(asset)).isNotEmpty())

    println("Media library synthetic wire and bounds checks passed")
}
