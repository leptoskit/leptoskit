package dev.leptoskit.plugins.livephoto

import dev.leptoskit.bridge.PluginException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Synthetic wire checks without loading a Live Photo. */
fun main() {
    val create =
        ByteBuffer.allocate(4 + "ph://a".toByteArray().size).order(ByteOrder.LITTLE_ENDIAN).apply {
            val utf8 = "ph://a".toByteArray(Charsets.UTF_8)
            putInt(utf8.size)
            put(utf8)
            flip()
        }
    assertEquals("ph://a", LivePhotoWire.create(create))
    val start =
        ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).apply {
            putInt(3)
            putInt(1)
            flip()
        }
    val decoded = LivePhotoWire.startPlayback(start)
    assertEquals(3, decoded.first)
    assertEquals(1, decoded.second)
    val status = LivePhotoWire.status(active = true, playing = false)
    assertEquals(12, status.size)
    try {
        LivePhotoWire.startPlayback(
            ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).apply {
                putInt(1)
                putInt(9)
                flip()
            },
        )
        error("expected invalid argument")
    } catch (error: PluginException) {
        assertEquals(PluginException.INVALID_ARGUMENT, error.code)
    }
}

class LivePhotoWireTest {
    @Test
    fun boundedCreateAndPlayback() {
        main()
        assertTrue(true)
    }
}
