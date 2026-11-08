package dev.leptoskit.plugins.appintegrity

import java.nio.ByteBuffer
import java.nio.ByteOrder

fun main() {
    require(AppIntegrityWire.capability(Capability.Unavailable).contentEquals(byteArrayOf(2, 0, 0, 0)))
    val challenge =
        ByteBuffer
            .allocate(8)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(4)
            .put("abcd".toByteArray())
            .array()
    require(AppIntegrityWire.challenge(ByteBuffer.wrap(challenge)).contentEquals("abcd".toByteArray()))
    require(AppIntegrityWire.attestationUnavailable().contentEquals(byteArrayOf(0, 0, 0, 0)))
    println("AppIntegrityWireTest ok")
}
