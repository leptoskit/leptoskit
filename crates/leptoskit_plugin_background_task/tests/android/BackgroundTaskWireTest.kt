package dev.leptoskit.plugins.backgroundtask

import dev.leptoskit.bridge.PluginException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Synthetic wire checks without scheduling WorkManager. */
fun main() {
    val register =
        ByteBuffer.allocate(4 + 4 + "sync".toByteArray().size).order(ByteOrder.LITTLE_ENDIAN).apply {
            putInt(900)
            val utf8 = "sync".toByteArray(Charsets.UTF_8)
            putInt(utf8.size)
            put(utf8)
            flip()
        }
    val decoded = BackgroundTaskWire.register(register)
    assertEquals("sync", decoded.name)
    assertEquals(900, decoded.intervalSecs)
    assertEquals(4, BackgroundTaskWire.status(Status.Available).size)
    try {
        BackgroundTaskWire.register(
            ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).apply {
                putInt(60)
                putInt(0)
                flip()
            },
        )
        error("expected invalid argument")
    } catch (error: PluginException) {
        assertEquals(PluginException.INVALID_ARGUMENT, error.code)
    }
}

class BackgroundTaskWireTest {
    @Test
    fun boundedRegisterAndStatus() {
        main()
        assertTrue(true)
    }
}
