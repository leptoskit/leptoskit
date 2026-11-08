package dev.leptoskit.plugins.videothumbnails

import dev.leptoskit.bridge.PluginException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Synthetic wire checks without decoding a video. */
fun main() {
    val fixed =
        ByteBuffer.allocate(8 + 4 + 4 + "file:///tmp/a.mp4".toByteArray().size).order(ByteOrder.LITTLE_ENDIAN).apply {
            putInt(1500) // low
            putInt(0) // high
            putInt(80)
            val utf8 = "file:///tmp/a.mp4".toByteArray(Charsets.UTF_8)
            putInt(utf8.size)
            put(utf8)
            flip()
        }
    val decoded = VideoThumbnailsWire.getThumbnail(fixed)
    assertEquals("file:///tmp/a.mp4", decoded.uri)
    assertEquals(1500L, decoded.timeMs)
    assertEquals(80, decoded.quality)
    val reply = VideoThumbnailsWire.thumbnail("file:///tmp/t.jpg", 320, 180)
    assertTrue(reply.isNotEmpty())
    try {
        VideoThumbnailsWire.getThumbnail(
            ByteBuffer.allocate(16).order(ByteOrder.LITTLE_ENDIAN).apply {
                putInt(0)
                putInt(0)
                putInt(0)
                putInt(0)
                flip()
            },
        )
        error("expected invalid argument")
    } catch (error: PluginException) {
        assertEquals(PluginException.INVALID_ARGUMENT, error.code)
    }
}

class VideoThumbnailsWireTest {
    @Test
    fun boundedGetThumbnail() {
        main()
        assertTrue(true)
    }
}
