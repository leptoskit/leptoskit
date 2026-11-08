package dev.leptoskit.plugins.print

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

internal sealed class PrintJob {
    data class Html(val html: String) : PrintJob()

    data class PdfUri(val uri: String) : PrintJob()

    companion object {
        fun decode(payload: ByteBuffer): PrintJob {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val kind = readU32(bytes)
            val body = readString(bytes)

            if (bytes.hasRemaining()) throw invalid()

            return when (kind) {
                0 -> html(body)
                1 -> pdfUri(body)
                else -> throw invalid()
            }
        }

        private fun html(value: String): PrintJob {
            if (value.length > MAX_HTML_BYTES) throw PluginException(PluginException.TOO_LARGE)

            if ('\u0000' in value) throw invalid()

            return Html(value)
        }

        private fun pdfUri(value: String): PrintJob {
            if (value.length > MAX_URI_BYTES) throw PluginException(PluginException.TOO_LARGE)

            requireGraphic(value)
            requireEscapes(value)

            val separator = value.indexOf("://")
            if (separator < 0) throw invalid()

            val scheme = value.substring(0, separator)
            val rest = value.substring(separator + 3)

            when (scheme) {
                "https", "http" -> {
                    val authority = rest.split('/', '?', '#').firstOrNull().orEmpty()
                    if (authority.isEmpty() || '@' in authority) throw invalid()
                }
                "file" -> {
                    if (!rest.startsWith("/") || '@' in rest) throw invalid()
                }
                else -> throw invalid()
            }

            return PdfUri(value)
        }

        private fun requireGraphic(value: String) {
            if (value.any { it.code !in FIRST_PRINTABLE..LAST_PRINTABLE || it == '\\' }) {
                throw invalid()
            }
        }

        private fun requireEscapes(value: String) {
            var index = 0

            while (index < value.length) {
                if (value[index] != '%') {
                    index++
                    continue
                }

                val first = value.getOrNull(index + 1)
                val second = value.getOrNull(index + 2)

                if (first == null || second == null || !isHex(first) || !isHex(second)) {
                    throw invalid()
                }

                index += 3
            }
        }

        private fun isHex(value: Char): Boolean =
            value in '0'..'9' || value in 'a'..'f' || value in 'A'..'F'

        private fun readU32(bytes: ByteBuffer): Int {
            if (bytes.remaining() < 4) throw invalid()

            val value = bytes.int

            if (value < 0) throw invalid()

            return value
        }

        private fun readString(bytes: ByteBuffer): String {
            val size = readU32(bytes)

            if (bytes.remaining() < size) throw invalid()

            val slice = bytes.slice().limit(size).order(ByteOrder.LITTLE_ENDIAN)
            bytes.position(bytes.position() + size)

            return try {
                Charsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(slice)
                    .toString()
            } catch (error: CharacterCodingException) {
                throw PluginException(PluginException.INVALID_ARGUMENT, error)
            }
        }

        private fun invalid() = PluginException(PluginException.INVALID_ARGUMENT)

        private const val MAX_HTML_BYTES = 256 * 1024
        private const val MAX_URI_BYTES = 8192
        private const val FIRST_PRINTABLE = 0x21
        private const val LAST_PRINTABLE = 0x7e
    }
}
