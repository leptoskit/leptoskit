package dev.leptoskit.plugins.camera

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder

@RunWith(AndroidJUnit4::class)
class CameraPluginTest {
    @Test
    fun wireCreateTorchTakeAndStatus() {
        assertEquals(0, CameraWire.create(facing(0)))
        assertEquals(1, CameraWire.create(facing(1)))
        val torch = CameraWire.torch(words(3, 1))
        assertEquals(3, torch.first)
        assertTrue(torch.second)
        val take = CameraWire.takePicture(words(4, 1024))
        assertEquals(4, take.first)
        assertEquals(1024, take.second)
        val status = CameraWire.status(0, true, false, true)
        assertEquals(20, status.size)
        val asset =
            CameraWire.asset(
                uri = "file:///tmp/a.jpg",
                mime = "image/jpeg",
                width = 10,
                height = 8,
                size = 12,
            )
        assertTrue(asset.isNotEmpty())
    }

    @Test
    fun rejectsBadFacingAndOversizeBudget() {
        try {
            CameraWire.create(facing(9))
            error("expected invalid argument")
        } catch (error: PluginException) {
            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
        }
        try {
            CameraWire.takePicture(words(1, MAX_BYTES + 1))
            error("expected too large")
        } catch (error: PluginException) {
            assertEquals(PluginException.TOO_LARGE, error.code)
        }
    }

    @Test
    fun createWithoutPermissionFailsHonestly() =
        runBlocking {
            val plugin = CameraPlugin(ApplicationProvider.getApplicationContext())
            try {
                plugin.invoke(0, facing(0))
                error("expected permission denied")
            } catch (error: PluginException) {
                assertEquals(PluginException.PERMISSION_DENIED, error.code)
            }
            plugin.close()
        }

    private fun facing(value: Int): ByteBuffer = words(value)

    private fun words(vararg values: Int): ByteBuffer {
        val buffer = ByteBuffer.allocate(values.size * 4).order(ByteOrder.LITTLE_ENDIAN)
        values.forEach { buffer.putInt(it) }
        buffer.flip()
        return buffer
    }
}
