package dev.leptoskit.plugins.network

/**
 * Maps Android transport codes to the wire's transport bitset, as Flutter's
 * connectivity_plus lists them. Pure, so it runs without a device.
 */
internal object NetworkTransports {
    /** Wire bits, matching `Transport` in the Rust crate. */
    const val WIFI = 1 shl 0
    const val CELLULAR = 1 shl 1
    const val ETHERNET = 1 shl 2
    const val BLUETOOTH = 1 shl 3
    const val VPN = 1 shl 4
    const val SATELLITE = 1 shl 5
    const val OTHER = 1 shl 6

    /** Every bit the wire defines. */
    const val MASK = (1 shl 7) - 1

    /** `NetworkCapabilities.TRANSPORT_*` values, as literals so this compiles off-device. */
    private const val TRANSPORT_CELLULAR = 0
    private const val TRANSPORT_WIFI = 1
    private const val TRANSPORT_BLUETOOTH = 2
    private const val TRANSPORT_ETHERNET = 3
    private const val TRANSPORT_VPN = 4
    private const val TRANSPORT_WIFI_AWARE = 5
    private const val TRANSPORT_SATELLITE = 10

    /** Codes probed with `hasTransport`; unknown ones answer false. Covers LoWPAN, USB, Thread and later ones too. */
    val CODES = (0..31).toList()

    /** Wire primary-connection codes that name a transport. */
    private const val PRIMARY_WIFI = 2
    private const val PRIMARY_CELLULAR = 3
    private const val PRIMARY_ETHERNET = 4
    private const val PRIMARY_BLUETOOTH = 5
    private const val PRIMARY_VPN = 6
    private const val PRIMARY_OTHER = 7

    /** The bitset for every transport present; transports without a code of their own become other. */
    fun bits(
        present: Iterable<Int>,
        primary: Int,
    ): Int {
        val reported = present.fold(0) { bits, transport -> bits or bit(transport) }

        return reported or primaryBit(primary)
    }

    private fun bit(transport: Int): Int =
        when (transport) {
            TRANSPORT_WIFI, TRANSPORT_WIFI_AWARE -> WIFI
            TRANSPORT_CELLULAR -> CELLULAR
            TRANSPORT_ETHERNET -> ETHERNET
            TRANSPORT_BLUETOOTH -> BLUETOOTH
            TRANSPORT_VPN -> VPN
            TRANSPORT_SATELLITE -> SATELLITE
            else -> OTHER
        }

    /** The primary connection's own bit, which the wire requires in the set. */
    private fun primaryBit(primary: Int): Int =
        when (primary) {
            PRIMARY_WIFI -> WIFI
            PRIMARY_CELLULAR -> CELLULAR
            PRIMARY_ETHERNET -> ETHERNET
            PRIMARY_BLUETOOTH -> BLUETOOTH
            PRIMARY_VPN -> VPN
            PRIMARY_OTHER -> OTHER
            else -> 0
        }
}
