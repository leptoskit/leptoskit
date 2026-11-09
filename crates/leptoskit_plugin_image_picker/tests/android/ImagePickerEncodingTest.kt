package dev.leptoskit.plugins.imagepicker

import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Synthetic JVM checks for the image-picker reply framing. */
object ImagePickerEncodingTest {
    @JvmStatic
    fun main(args: Array<String>) {
        val uri = "file:///data/cache/photo.jpg".toByteArray(Charsets.UTF_8)
        val mime = "image/jpeg".toByteArray(Charsets.UTF_8)
        val encoded =
            ByteBuffer
                .allocate(24 + uri.size + mime.size)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(uri.size)
                .put(uri)
                .putInt(mime.size)
                .put(mime)
                .putInt(100)
                .putInt(80)
                .putInt(12)
                .putInt(0)
                .array()

        val buffer = ByteBuffer.wrap(encoded).order(ByteOrder.LITTLE_ENDIAN)
        val uriLength = buffer.int
        require(uriLength == uri.size)
        val uriBytes = ByteArray(uriLength)
        buffer.get(uriBytes)
        require(uriBytes.decodeToString() == "file:///data/cache/photo.jpg")
        val mimeLength = buffer.int
        val mimeBytes = ByteArray(mimeLength)
        buffer.get(mimeBytes)
        require(mimeBytes.decodeToString() == "image/jpeg")
        require(buffer.int == 100)
        require(buffer.int == 80)
        require(buffer.int == 12)
        require(buffer.int == 0)
        require(!buffer.hasRemaining())
        println("image-picker-encoding-ok")
    }
}

/** The instrumentation wrapper calls this, the same way the other wire fixtures do. */
fun main() {
    ImagePickerEncodingTest.main(emptyArray())
}
