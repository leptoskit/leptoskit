package dev.leptoskit.plugins.asset

import java.nio.ByteBuffer
import java.nio.ByteOrder

fun main() {
    require(AssetWire.capability(Capability.Unavailable).contentEquals(byteArrayOf(2, 0, 0, 0)))
    val name = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).putInt(4).put("icon".toByteArray()).array()
    require(AssetWire.name(ByteBuffer.wrap(name)) == "icon")
    println("AssetWireTest ok")
}
