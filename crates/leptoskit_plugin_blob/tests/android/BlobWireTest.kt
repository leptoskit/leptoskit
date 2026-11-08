package dev.leptoskit.plugins.blob
import java.nio.ByteBuffer
import java.nio.ByteOrder
fun main() {
    val create = ByteBuffer.allocate(6).order(ByteOrder.LITTLE_ENDIAN).putInt(2).put("hi".toByteArray()).array()
    require(BlobWire.create(ByteBuffer.wrap(create)).contentEquals("hi".toByteArray()))
    println("BlobWireTest ok")
}
