package dev.leptoskit.plugins.webbrowser

import dev.leptoskit.bridge.PluginException
import java.net.URI
import java.net.URISyntaxException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

internal data class WebBrowserOpen(
    val url: String,
) {
    companion object {
        fun decode(payload: ByteBuffer): WebBrowserOpen {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val value = readString(bytes)

            if (bytes.hasRemaining()) throw invalid()

            return open(value)
        }

        fun open(value: String): WebBrowserOpen {
            if (value.length > MAX_URL_BYTES) throw PluginException(PluginException.TOO_LARGE)

            val uri = parse(value)

            if (uri.scheme !in WEB_SCHEMES) throw invalid()

            requireDestination(uri)
            requireVisible(value)
            requireEscapes(value)

            return WebBrowserOpen(value)
        }

        private fun requireDestination(uri: URI) {
            if (uri.host.isNullOrEmpty() || uri.rawAuthority.isNullOrEmpty()) throw invalid()

            if ('@' in uri.rawAuthority || uri.userInfo != null) throw invalid()
        }

        private fun requireVisible(value: String) {
            if (value.any { it.code !in FIRST_PRINTABLE..LAST_PRINTABLE || it == '\\' }) throw invalid()
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

        private fun parse(value: String): URI =
            try {
                URI(value)
            } catch (error: URISyntaxException) {
                throw PluginException(PluginException.INVALID_ARGUMENT, error)
            }

        private fun readString(bytes: ByteBuffer): String {
            if (bytes.remaining() < 4) throw invalid()

            val size = bytes.int

            if (size < 0 || bytes.remaining() < size) throw invalid()

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

        private const val MAX_URL_BYTES = 8192
        private const val FIRST_PRINTABLE = 33
        private const val LAST_PRINTABLE = 126
        private val WEB_SCHEMES = setOf("http", "https")
    }
}
