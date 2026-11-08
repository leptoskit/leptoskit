package dev.leptoskit.plugins.networkinfo

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkAddress
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.wifi.WifiInfo
import android.net.wifi.WifiManager
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.net.Inet4Address
import java.net.Inet6Address
import java.nio.ByteBuffer

/**
 * Reads the Wi-Fi name, BSSID, and the Wi-Fi network's addresses. It never
 * asks for permission: without location access Android withholds the name
 * and BSSID, and this plugin then reports them as absent.
 */
class NetworkInfoPlugin(
    context: Context,
) : NativePlugin {
    override val name = "network-info"
    override val methods = intArrayOf(SNAPSHOT)
    private val connectivity = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
    private val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method != SNAPSHOT) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return snapshot()
    }

    override fun close() {
        closed = true
    }

    private fun snapshot(): ByteArray {
        val network = wifiNetwork()
        val link = network?.let { withheldOnSecurity { connectivity?.getLinkProperties(it) } }
        val info = if (network == null) null else connectionInfo()
        val ipv4 = preferred(link, Inet4Address::class.java)
        val ipv6 = preferred(link, Inet6Address::class.java)

        return NetworkInfoWire.encode(
            NetworkInfoWire.Fields(
                ssid = NetworkInfoWire.ssid(info?.ssid),
                bssid = NetworkInfoWire.bssid(info?.bssid),
                ipv4 = ipv4?.address?.address,
                ipv6 = ipv6?.address?.address,
                subnetMask = ipv4?.let { NetworkInfoWire.mask(it.prefixLength) },
                broadcast = ipv4?.let { NetworkInfoWire.broadcast(it.address.address, it.prefixLength) },
                gateway = gateway(link),
            ),
        )
    }

    /** The Wi-Fi network, even when a VPN or cellular network is the default. */
    @Suppress("DEPRECATION")
    private fun wifiNetwork(): Network? {
        val manager = connectivity ?: return null

        return withheldOnSecurity {
            (listOfNotNull(manager.activeNetwork) + manager.allNetworks).firstOrNull { network ->
                manager.getNetworkCapabilities(network)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true
            }
        }
    }

    /** `WifiManager.getConnectionInfo` still reports the SSID to apps holding location access. */
    @Suppress("DEPRECATION")
    private fun connectionInfo(): WifiInfo? = withheldOnSecurity { wifi?.connectionInfo }

    /** The first address of the family, preferring a routable one to a link-local one. */
    private fun preferred(
        link: LinkProperties?,
        family: Class<*>,
    ): LinkAddress? {
        val candidates =
            link?.linkAddresses.orEmpty().filter {
                family.isInstance(it.address) && !it.address.isLoopbackAddress && !it.address.isAnyLocalAddress
            }

        return candidates.firstOrNull { !it.address.isLinkLocalAddress } ?: candidates.firstOrNull()
    }

    /** The IPv4 default route's next hop on the Wi-Fi network. */
    private fun gateway(link: LinkProperties?): ByteArray? =
        link
            ?.routes
            .orEmpty()
            .firstOrNull { it.isDefaultRoute && it.gateway is Inet4Address && it.gateway?.isAnyLocalAddress == false }
            ?.gateway
            ?.address

    /** Android answers a missing permission with a SecurityException; that is a withheld value. */
    private inline fun <T> withheldOnSecurity(read: () -> T?): T? =
        try {
            read()
        } catch (_: SecurityException) {
            null
        }

    private companion object {
        /** Answers the current reading; takes no arguments. */
        const val SNAPSHOT = 0
    }
}
