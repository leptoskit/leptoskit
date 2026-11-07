package dev.leptoskit.plugins.application

import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.security.MessageDigest

/** Everything the plugin reports. `null` means unknown. */
internal data class ApplicationFacts(
    val id: String? = null,
    val name: String? = null,
    val version: String? = null,
    val build: String? = null,
    val buildSignature: String? = null,
    val installerStore: String? = null,
    val installTime: Long? = null,
    val updateTime: Long? = null,
)

/** The little-endian wire shared with Rust and Swift. */
internal object ApplicationWire {
    const val VERSION = 2
    const val MAX_TEXT_BYTES = 4096
    const val MAX_INSTALLER_BYTES = 256

    /** Room for the header, strings and times of a typical answer. */
    private const val RESULT_CAPACITY = 256

    /** Hexadecimal digits for [sha256]. */
    private const val HEX_DIGITS = "0123456789ABCDEF"

    /** The low four bits of a byte. */
    private const val NIBBLE = 0x0F

    /** The SHA-256 digest of `bytes` as 64 uppercase hexadecimal digits. */
    fun sha256(bytes: ByteArray): String =
        buildString {
            for (byte in MessageDigest.getInstance("SHA-256").digest(bytes)) {
                val value = byte.toInt()
                append(HEX_DIGITS[(value shr 4) and NIBBLE])
                append(HEX_DIGITS[value and NIBBLE])
            }
        }

    fun snapshot(facts: ApplicationFacts): ByteArray {
        val output = ByteArrayOutputStream(RESULT_CAPACITY)
        val word = ByteBuffer.allocate(Long.SIZE_BYTES).order(ByteOrder.LITTLE_ENDIAN)

        fun u32(value: Int) {
            word.clear()
            output.write(word.putInt(value).array(), 0, Int.SIZE_BYTES)
        }

        u32(VERSION)

        for ((text, max) in listOf(
            facts.id to MAX_TEXT_BYTES,
            facts.name to MAX_TEXT_BYTES,
            facts.version to MAX_TEXT_BYTES,
            facts.build to MAX_TEXT_BYTES,
            facts.buildSignature to MAX_TEXT_BYTES,
            facts.installerStore to MAX_INSTALLER_BYTES,
        )) {
            val bytes = (text ?: "").toByteArray(Charsets.UTF_8)

            if (bytes.size > max) throw PluginException(PluginException.TOO_LARGE)
            u32(bytes.size)
            output.write(bytes)
        }

        for (time in listOf(facts.installTime, facts.updateTime)) {
            val millis = time?.takeIf { it >= 0 }
            u32(if (millis == null) 0 else 1)
            word.clear()
            output.write(word.putLong(millis ?: 0).array(), 0, Long.SIZE_BYTES)
        }

        return output.toByteArray()
    }
}
