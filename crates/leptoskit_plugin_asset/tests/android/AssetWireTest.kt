package dev.leptoskit.plugins.asset

import java.nio.ByteBuffer
import java.nio.ByteOrder

fun main() {
    require(AssetWire.capability(Capability.Unavailable).contentEquals(byteArrayOf(2, 0, 0, 0)))
    val name =
        ByteBuffer
            .allocate(8)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(4)
            .put("icon".toByteArray())
            .array()
    require(AssetWire.name(ByteBuffer.wrap(name)) == "icon")

    val nul =
        ByteBuffer
            .allocate(7)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(3)
            .put(byteArrayOf(0x61, 0x00, 0x62))
            .array()
    runCatching { AssetWire.name(ByteBuffer.wrap(nul)) }.exceptionOrNull()
        ?: error("expected NUL rejection")

    val trailing =
        ByteBuffer
            .allocate(9)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(4)
            .put("icon".toByteArray())
            .put(0)
            .array()
    runCatching { AssetWire.name(ByteBuffer.wrap(trailing)) }.exceptionOrNull()
        ?: error("expected trailing rejection")
    println("AssetWireTest ok")
}
