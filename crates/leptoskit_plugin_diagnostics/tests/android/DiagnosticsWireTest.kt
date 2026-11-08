package dev.leptoskit.plugins.diagnostics
import java.nio.ByteBuffer
import java.nio.ByteOrder
fun main() {
    val name = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).putInt(4).put("boot".toByteArray()).array()
    require(DiagnosticsWire.name(ByteBuffer.wrap(name)) == "boot")
    println("DiagnosticsWireTest ok")
}
