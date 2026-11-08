package dev.leptoskit.plugins.maps

import dev.leptoskit.bridge.PluginException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Synthetic wire checks without creating a Google Map. */
fun main() {
    MapsWire.create(ByteBuffer.allocate(0))
    val camera = MapCamera.validated(37.77, -122.42, 12.0)
    val setCamera =
        ByteBuffer.allocate(28).order(ByteOrder.LITTLE_ENDIAN).apply {
            putInt(3)
            putDouble(camera.latitude)
            putDouble(camera.longitude)
            putDouble(camera.zoom)
            flip()
        }
    val decoded = MapsWire.setCamera(setCamera)
    assertEquals(3, decoded.first)
    assertEquals(camera, decoded.second)
    val status = MapsWire.status(MapCamera.DEFAULT, 1, true)
    assertEquals(36, status.size)
    try {
        MapsWire.setMarkers(
            ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).apply {
                putInt(1)
                putInt(MapsWire.MAX_MARKERS + 1)
                flip()
            },
        )
        error("expected too large")
    } catch (error: PluginException) {
        assertEquals(PluginException.TOO_LARGE, error.code)
    }
}

class MapsWireTest {
    @Test
    fun boundedCameraAndMarkers() {
        main()
        assertTrue(true)
    }
}
