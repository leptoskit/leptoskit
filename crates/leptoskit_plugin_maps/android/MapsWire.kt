package dev.leptoskit.plugins.maps

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CodingErrorAction

internal data class MapCamera(
    val latitude: Double,
    val longitude: Double,
    val zoom: Double,
) {
    companion object {
        val DEFAULT = MapCamera(0.0, 0.0, 2.0)

        fun validated(
            latitude: Double,
            longitude: Double,
            zoom: Double,
        ): MapCamera {
            if (!latitude.isFinite() || latitude !in -90.0..90.0) throw invalid()
            if (!longitude.isFinite() || longitude !in -180.0..180.0) throw invalid()
            if (!zoom.isFinite() || zoom !in 0.0..22.0) throw invalid()
            return MapCamera(latitude, longitude, zoom)
        }
    }
}

internal data class MapMarker(
    val latitude: Double,
    val longitude: Double,
    val title: String,
)

internal object MapsWire {
    const val MAX_MARKERS = 32
    const val MAX_TITLE_BYTES = 128
    const val WIRE_VERSION = 1

    fun create(payload: ByteBuffer) {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (bytes.hasRemaining()) throw invalid()
    }

    fun session(payload: ByteBuffer): Int {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val id = readU32(bytes)
        if (bytes.hasRemaining()) throw invalid()
        return id
    }

    fun setCamera(payload: ByteBuffer): Pair<Int, MapCamera> {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val id = readU32(bytes)
        val camera = readCamera(bytes)
        if (bytes.hasRemaining()) throw invalid()
        return id to camera
    }

    fun setMarkers(payload: ByteBuffer): Pair<Int, List<MapMarker>> {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val id = readU32(bytes)
        val count = readU32(bytes)
        if (count > MAX_MARKERS) throw PluginException(PluginException.TOO_LARGE)
        val markers = List(count) { readMarker(bytes) }
        if (bytes.hasRemaining()) throw invalid()
        return id to markers
    }

    fun status(
        camera: MapCamera,
        markerCount: Int,
        active: Boolean,
    ): ByteArray {
        val buffer = ByteBuffer.allocate(36).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(WIRE_VERSION)
        putF64(buffer, camera.latitude)
        putF64(buffer, camera.longitude)
        putF64(buffer, camera.zoom)
        buffer.putInt(markerCount)
        buffer.putInt(if (active) 1 else 0)
        return buffer.array()
    }

    fun sessionId(id: Int): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(id)
        return buffer.array()
    }

    private fun readCamera(bytes: ByteBuffer): MapCamera = MapCamera.validated(readF64(bytes), readF64(bytes), readF64(bytes))

    private fun readMarker(bytes: ByteBuffer): MapMarker {
        val latitude = readF64(bytes)
        val longitude = readF64(bytes)
        val title = readString(bytes)
        if (!latitude.isFinite() || latitude !in -90.0..90.0) throw invalid()
        if (!longitude.isFinite() || longitude !in -180.0..180.0) throw invalid()
        if ('\u0000' in title || title.toByteArray(Charsets.UTF_8).size > MAX_TITLE_BYTES) {
            throw invalid()
        }
        return MapMarker(latitude, longitude, title)
    }
}

private fun readU32(bytes: ByteBuffer): Int {
    if (bytes.remaining() < 4) throw invalid()
    val value = bytes.int
    if (value < 0) throw invalid()
    return value
}

private fun readF64(bytes: ByteBuffer): Double {
    if (bytes.remaining() < 8) throw invalid()
    return bytes.double
}

private fun putF64(
    buffer: ByteBuffer,
    value: Double,
) {
    buffer.putDouble(value)
}

private fun readString(bytes: ByteBuffer): String {
    val length = readU32(bytes)
    if (bytes.remaining() < length) throw invalid()
    val slice = ByteArray(length)
    bytes.get(slice)
    return try {
        Charsets.UTF_8
            .newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
            .decode(ByteBuffer.wrap(slice))
            .toString()
    } catch (_: CharacterCodingException) {
        throw invalid()
    }
}

private fun invalid(): Nothing = throw PluginException(PluginException.INVALID_ARGUMENT)
