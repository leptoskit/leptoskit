package dev.leptoskit.plugins.devclient

import dev.leptoskit.bridge.PluginException
import java.net.Inet4Address
import java.net.Inet6Address
import java.net.InetAddress
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CodingErrorAction

internal object DevClientWire {
    fun status(available: Boolean): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(if (available) 1 else 2)
        return buffer.array()
    }

    fun guestUrl(payload: ByteBuffer): String {
        val text = text(payload)
        return canonical(text)
    }

    fun text(payload: ByteBuffer): String {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (bytes.remaining() < 4) throw invalid()
        val length = bytes.int
        if (length < 0 || bytes.remaining() != length) throw invalid()
        val encoded = ByteArray(length)
        bytes.get(encoded)
        return try {
            Charsets.UTF_8
                .newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(encoded))
                .toString()
        } catch (_: CharacterCodingException) {
            throw invalid()
        }
    }

    fun encode(text: String): ByteArray {
        val encoded = text.toByteArray(Charsets.UTF_8)
        val buffer = ByteBuffer.allocate(4 + encoded.size).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(encoded.size)
        buffer.put(encoded)
        return buffer.array()
    }

    /**
     * The same address [leptoskit_plugin_dev_client::guest_url] accepts, for
     * either phone. A base address becomes the Android guest.
     */
    fun canonical(text: String): String = tryPlatform(text, "android") ?: tryPlatform(text, "ios") ?: throw invalid()

    private fun tryPlatform(
        text: String,
        platform: String,
    ): String? =
        try {
            one(text, platform)
        } catch (_: PluginException) {
            null
        }

    private fun one(
        scanned: String,
        platform: String,
    ): String {
        val text = scanned.trim()
        if (text.isEmpty() || text.any { it == '?' || it == '#' || it.isWhitespace() }) throw invalid()
        val scheme =
            when {
                text.startsWith("http://") -> "http"
                text.startsWith("https://") -> "https"
                else -> throw invalid()
            }
        val rest = text.substring(scheme.length + 3)
        if (rest.contains('@')) throw invalid()
        val slash = rest.indexOf('/')
        val authority = if (slash < 0) rest else rest.substring(0, slash)
        val path = if (slash < 0) "" else rest.substring(slash).trimEnd('/')
        val (host, port) = splitHost(authority, scheme)
        val guest = "/_leptoskit/dev/$platform/guest"
        if (path.isNotEmpty() && path != "/_leptoskit/dev" && path != guest) throw invalid()
        return "$scheme://$host:$port$guest"
    }

    private fun splitHost(
        authority: String,
        scheme: String,
    ): Pair<String, Int> {
        if (authority.isEmpty()) throw invalid()
        val (host, explicit) =
            if (authority.startsWith("[")) {
                val end = authority.indexOf(']')
                if (end < 0) throw invalid()
                val host = authority.substring(1, end)
                val after = authority.substring(end + 1)
                val port =
                    when {
                        after.isEmpty() -> null
                        after.startsWith(":") -> port(after.substring(1))
                        else -> throw invalid()
                    }
                host to port
            } else if (authority.contains(':')) {
                val index = authority.lastIndexOf(':')
                val host = authority.substring(0, index)
                if (host.isEmpty() || host.contains(':')) throw invalid()
                host to port(authority.substring(index + 1))
            } else {
                authority to null
            }
        if (host.isEmpty() || !devHost(host)) throw invalid()
        val rendered = if (host.contains(':')) "[$host]" else host
        return rendered to (explicit ?: if (scheme == "https") 443 else 80)
    }

    private fun port(text: String): Int {
        val value = text.toIntOrNull() ?: throw invalid()
        if (value !in 1..65535) throw invalid()
        return value
    }

    private fun devHost(host: String): Boolean {
        if (host.equals("localhost", ignoreCase = true)) return true
        if (host.endsWith(".local", ignoreCase = true)) {
            val name = host.dropLast(".local".length)
            if (name.isEmpty()) return false
            return name.split('.').all { label ->
                label.length in 1..63 &&
                    !label.startsWith('-') &&
                    !label.endsWith('-') &&
                    label.all { it.isLetterOrDigit() || it == '-' }
            }
        }
        val address =
            try {
                literal(host) ?: return false
            } catch (_: IllegalArgumentException) {
                return false
            }
        return when (address) {
            is Inet4Address -> {
                val octets = address.address
                val a = octets[0].toInt() and 0xff
                val b = octets[1].toInt() and 0xff
                a != 0 &&
                    a != 255 &&
                    a !in 224..239 &&
                    (
                        a == 127 ||
                            a == 10 ||
                            (a == 169 && b == 254) ||
                            (a == 172 && b in 16..31) ||
                            (a == 192 && b == 168)
                    )
            }

            is Inet6Address -> {
                val bytes = address.address
                val first = bytes[0].toInt() and 0xff
                val second = bytes[1].toInt() and 0xff
                val unspecified = bytes.all { it.toInt() == 0 }
                val loopback = bytes.dropLast(1).all { it.toInt() == 0 } && bytes.last().toInt() == 1
                !unspecified &&
                    (loopback || (first and 0xfe == 0xfc) || (first == 0xfe && second and 0xc0 == 0x80))
            }

            else -> {
                false
            }
        }
    }

    /** Parses an address literal. A name is not resolved. */
    private fun literal(host: String): InetAddress? {
        val v4 = host.split('.')
        if (v4.size == 4 && v4.all { octet -> octet.toIntOrNull() in 0..255 && octet == octet.toInt().toString() }) {
            return InetAddress.getByAddress(v4.map { it.toInt().toByte() }.toByteArray())
        }
        if (!host.contains(':')) return null
        return InetAddress.getByName(host).takeIf { it is Inet6Address && it.hostAddress?.contains(':') == true }
    }
}

private fun invalid(): Nothing = throw PluginException(PluginException.INVALID_ARGUMENT)
