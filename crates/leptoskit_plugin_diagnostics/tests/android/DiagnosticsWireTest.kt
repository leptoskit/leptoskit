package dev.leptoskit.plugins.diagnostics

import java.nio.ByteBuffer
import java.nio.ByteOrder

fun main() {
    DiagnosticsWire.empty(ByteBuffer.allocate(0))
    val name =
        ByteBuffer
            .allocate(8)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(4)
            .put("boot".toByteArray())
            .array()
    require(DiagnosticsWire.name(ByteBuffer.wrap(name)) == "boot")

    val nul =
        ByteBuffer
            .allocate(7)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(3)
            .put(byteArrayOf(0x61, 0x00, 0x62))
            .array()
    runCatching { DiagnosticsWire.name(ByteBuffer.wrap(nul)) }.exceptionOrNull()
        ?: error("expected NUL rejection")

    val id =
        ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(9)
            .array()
    require(DiagnosticsWire.id(ByteBuffer.wrap(id)) == 9)
    require(DiagnosticsWire.encodeId(9).contentEquals(id))
    require(DiagnosticsWire.capability(Capability.Available).contentEquals(byteArrayOf(1, 0, 0, 0)))
    println("DiagnosticsWireTest ok")
}
