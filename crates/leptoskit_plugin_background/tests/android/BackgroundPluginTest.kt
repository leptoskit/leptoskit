package dev.leptoskit.plugins.background

import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Validates the wire codec and status shape without starting a real service. */
class BackgroundPluginTest {
    @Test
    fun statusReportsStrictShapeAndStartRejectsMalformedPayloads() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val plugin = withContext(Dispatchers.Main) { BackgroundPlugin(context) }
            try {
                val status =
                    withContext(Dispatchers.Main) {
                        plugin.invoke(BackgroundPlugin.STATUS, ByteBuffer.allocate(0))
                    }
                assertEquals(12, status.size)
                val running = ByteBuffer.wrap(status).order(ByteOrder.LITTLE_ENDIAN).int
                assertEquals(0, running)

                val malformed = start(ByteArray(2))
                try {
                    withContext(Dispatchers.Main) {
                        plugin.invoke(BackgroundPlugin.START, ByteBuffer.wrap(malformed))
                    }
                    fail("Truncated start payload accepted")
                } catch (error: PluginException) {
                    assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                }

                try {
                    withContext(Dispatchers.Main) {
                        plugin.invoke(BackgroundPlugin.START, ByteBuffer.wrap(start(valid())))
                    }
                    fail("Start payload with a zero interval accepted")
                } catch (error: PluginException) {
                    assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                }
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
            }
        }

    @Test
    fun emptyPayloadsAreRequiredForStopAndStatus() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val plugin = withContext(Dispatchers.Main) { BackgroundPlugin(context) }
            try {
                for (method in intArrayOf(BackgroundPlugin.STOP, BackgroundPlugin.STATUS)) {
                    try {
                        withContext(Dispatchers.Main) {
                            plugin.invoke(method, ByteBuffer.wrap(byteArrayOf(1)))
                        }
                        fail("Method $method accepted a nonempty payload")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                    }
                }
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
            }
        }

    private fun valid(): ByteArray {
        val title = "Running".toByteArray(Charsets.UTF_8)
        val text = ByteArray(0)
        val buffer = ByteBuffer.allocate(8 + title.size + text.size).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(title.size).put(title)
        buffer.putInt(text.size).put(text)
        buffer.putInt(0)
        buffer.putInt(60)

        return buffer.array()
    }

    private fun start(payload: ByteArray): ByteArray {
        val title = "Running".toByteArray(Charsets.UTF_8)
        val buffer = ByteBuffer.allocate(8 + title.size + payload.size).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(title.size).put(title)
        buffer.putInt(payload.size).put(payload)

        return buffer.array()
    }
}
