package dev.leptoskit.plugins.video

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class VideoPluginTest {
    @Test
    fun createStatusAndRelease() =
        runBlocking {
            val plugin = VideoPlugin(ApplicationProvider.getApplicationContext())
            val created = plugin.invoke(0, source("https://example.com/a.mp4"))
            assertEquals(4, created.size)
            val id = ByteBuffer.wrap(created).order(ByteOrder.LITTLE_ENDIAN).int
            val status = plugin.invoke(8, player(id))
            assertEquals(16, status.size)
            plugin.invoke(9, player(id))
            try {
                plugin.invoke(8, player(id))
                error("expected invalid argument")
            } catch (error: PluginException) {
                assertEquals(PluginException.INVALID_ARGUMENT, error.code)
            }
            plugin.close()
        }

    @Test
    fun rejectsEmptyUrl() =
        runBlocking {
            val plugin = VideoPlugin(ApplicationProvider.getApplicationContext())
            try {
                plugin.invoke(0, source(""))
                error("expected invalid argument")
            } catch (error: PluginException) {
                assertEquals(PluginException.INVALID_ARGUMENT, error.code)
            }
            plugin.close()
        }

    private fun source(url: String): ByteBuffer {
        val bytes = url.toByteArray(Charsets.UTF_8)
        val buffer = ByteBuffer.allocate(8 + bytes.size).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(1)
        buffer.putInt(bytes.size)
        buffer.put(bytes)
        buffer.flip()
        return buffer
    }

    private fun player(id: Int): ByteBuffer =
        ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(id).also { it.flip() }
}
