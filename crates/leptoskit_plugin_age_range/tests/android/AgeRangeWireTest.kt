package dev.leptoskit.plugins.agerange

import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.system.exitProcess

fun main() {
    val status = AgeRangeWire.capability(Capability.Unavailable)
    require(status.contentEquals(byteArrayOf(2, 0, 0, 0)))

    val threshold =
        ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(18).array()
    require(AgeRangeWire.threshold(ByteBuffer.wrap(threshold)) == 18)
    require(AgeRangeWire.responseUnavailable().contentEquals(byteArrayOf(1, 0, 0, 0)))

    try {
        AgeRangeWire.threshold(ByteBuffer.wrap(ByteArray(0)))
        error("expected invalid empty threshold")
    } catch (_: Exception) {
    }
    println("AgeRangeWireTest ok")
}
