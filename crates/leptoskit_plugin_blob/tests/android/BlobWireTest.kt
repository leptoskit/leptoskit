package dev.leptoskit.plugins.blob

import java.nio.ByteBuffer
import java.nio.ByteOrder

fun main() {
    BlobWire.empty(ByteBuffer.allocate(0))
    runCatching { BlobWire.empty(ByteBuffer.wrap(byteArrayOf(0))) }.exceptionOrNull()
        ?: error("expected empty trailing rejection")

    val create =
        ByteBuffer
            .allocate(6)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(2)
            .put("hi".toByteArray())
            .array()
    require(BlobWire.create(ByteBuffer.wrap(create)).contentEquals("hi".toByteArray()))

    val read =
        ByteBuffer
            .allocate(12)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(1)
            .putInt(2)
            .putInt(3)
            .array()
    require(BlobWire.read(ByteBuffer.wrap(read)) == Triple(1, 2, 3))

    val id =
        ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(9)
            .array()
    require(BlobWire.id(ByteBuffer.wrap(id)) == 9)
    require(BlobWire.encodeId(9).contentEquals(id))
    require(BlobWire.capability(Capability.Available).contentEquals(byteArrayOf(1, 0, 0, 0)))
    require(
        BlobWire.encodeSlice("ab".toByteArray()).contentEquals(
            ByteBuffer
                .allocate(6)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(2)
                .put("ab".toByteArray())
                .array(),
        ),
    )
    println("BlobWireTest ok")
}
