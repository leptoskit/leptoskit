package dev.leptoskit.plugins.networkinfo

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * The 80-byte snapshot the Rust crate decodes. Absent fields stay all zero,
 * and nothing Android withholds is replaced by a placeholder. Pure, so it
 * runs without a device.
 */
internal object NetworkInfoWire {
    const val SNAPSHOT_BYTES = 80
    const val MAX_SSID_BYTES = 32

    /** What `WifiInfo.getSSID` answers when it withholds the name. */
    private const val UNKNOWN_SSID = "<unknown ssid>"

    /** What `WifiInfo.getBSSID` answers when it withholds the address. */
    private val REDACTED_BSSID = byteArrayOf(2, 0, 0, 0, 0, 0)

    private const val BSSID_BYTES = 6
    private const val IPV4_BYTES = 4
    private const val IPV4_BITS = 32

    /** The longest prefix that still leaves a broadcast address. */
    private const val LAST_BROADCAST_PREFIX = 30

    private const val SSID_LENGTH_OFFSET = 4
    private const val SSID_OFFSET = 8
    private const val BSSID_OFFSET = 40
    private const val IPV4_OFFSET = 48
    private const val IPV6_OFFSET = 52
    private const val SUBNET_MASK_OFFSET = 68
    private const val BROADCAST_OFFSET = 72
    private const val GATEWAY_OFFSET = 76

    /** Every optional field; each must already be valid or null. */
    class Fields(
        val ssid: ByteArray? = null,
        val bssid: ByteArray? = null,
        val ipv4: ByteArray? = null,
        val ipv6: ByteArray? = null,
        val subnetMask: ByteArray? = null,
        val broadcast: ByteArray? = null,
        val gateway: ByteArray? = null,
    )

    fun encode(fields: Fields): ByteArray {
        val output = ByteArray(SNAPSHOT_BYTES)
        val slots =
            listOf(
                SSID_OFFSET to fields.ssid,
                BSSID_OFFSET to fields.bssid,
                IPV4_OFFSET to fields.ipv4,
                IPV6_OFFSET to fields.ipv6,
                SUBNET_MASK_OFFSET to fields.subnetMask,
                BROADCAST_OFFSET to fields.broadcast,
                GATEWAY_OFFSET to fields.gateway,
            )
        var presence = 0

        slots.forEachIndexed { bit, (offset, bytes) ->
            if (bytes != null) {
                presence = presence or (1 shl bit)
                bytes.copyInto(output, offset)
            }
        }

        ByteBuffer
            .wrap(output)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(0, presence)
            .putInt(SSID_LENGTH_OFFSET, fields.ssid?.size ?: 0)
        return output
    }

    /**
     * Decodes `WifiInfo.getSSID`: UTF-8 in quotes, or raw bytes as hex digits.
     * The unknown marker, an empty name, and anything over 32 bytes are withheld.
     */
    fun ssid(raw: String?): ByteArray? {
        if (raw == null || raw == UNKNOWN_SSID) return null

        val bytes =
            if (raw.length >= 2 && raw.startsWith('"') && raw.endsWith('"')) {
                raw.substring(1, raw.length - 1).toByteArray(Charsets.UTF_8)
            } else {
                hex(raw)
            }

        return bytes?.takeIf { it.size in 1..MAX_SSID_BYTES }
    }

    /** Parses `aa:bb:cc:dd:ee:ff`, refusing the zero, redacted, and group addresses. */
    fun bssid(raw: String?): ByteArray? {
        val parts = raw?.split(':') ?: return null

        if (parts.size != BSSID_BYTES || parts.any { it.length !in 1..2 }) return null

        val octets = parts.map { it.toIntOrNull(radix = 16) ?: return null }
        val bytes = ByteArray(BSSID_BYTES) { octets[it].toByte() }
        val group = octets[0] and 1 == 1

        return bytes.takeUnless { group || it.all { byte -> byte.toInt() == 0 } || it.contentEquals(REDACTED_BSSID) }
    }

    /** The IPv4 subnet mask for a prefix length, or null outside 1..32. */
    fun mask(prefix: Int): ByteArray? {
        if (prefix !in 1..IPV4_BITS) return null

        return ipv4((-1L shl (IPV4_BITS - prefix)).toInt())
    }

    /** The broadcast address of `address`/`prefix`; /31 and /32 have none. */
    fun broadcast(
        address: ByteArray,
        prefix: Int,
    ): ByteArray? {
        if (address.size != IPV4_BYTES || prefix !in 1..LAST_BROADCAST_PREFIX) return null

        val host = (-1L shl (IPV4_BITS - prefix)).toInt().inv()

        return ipv4(ByteBuffer.wrap(address).int or host)
    }

    private fun ipv4(value: Int): ByteArray = ByteBuffer.allocate(IPV4_BYTES).putInt(value).array()

    private fun hex(text: String): ByteArray? {
        if (text.isEmpty() || text.length % 2 != 0) return null

        return ByteArray(text.length / 2) { index ->
            (text.substring(index * 2, index * 2 + 2).toIntOrNull(radix = 16) ?: return null).toByte()
        }
    }
}
