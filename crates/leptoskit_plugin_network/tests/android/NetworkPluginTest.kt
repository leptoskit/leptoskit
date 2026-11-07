package dev.leptoskit.plugins.network

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** The snapshot call, which takes no arguments. */
private const val SNAPSHOT = 0

/** The change call, which takes the caller's last reading. */
private const val WAIT_FOR_CHANGE = 1

/** Nine little-endian u32 fields. */
private const val SNAPSHOT_BYTES = 36

/** The eight-field snapshot of earlier releases, which the plugin now refuses. */
private const val LEGACY_SNAPSHOT_BYTES = 32

/** Wire primary-connection codes the transport tests use. */
private const val PRIMARY_NONE = 1
private const val PRIMARY_WIFI = 2
private const val PRIMARY_CELLULAR = 3
private const val PRIMARY_VPN = 6
private const val PRIMARY_OTHER = 7

/** `NetworkCapabilities.TRANSPORT_*` codes. */
private const val TRANSPORT_CELLULAR = 0
private const val TRANSPORT_WIFI = 1
private const val TRANSPORT_BLUETOOTH = 2
private const val TRANSPORT_ETHERNET = 3
private const val TRANSPORT_VPN = 4
private const val TRANSPORT_WIFI_AWARE = 5
private const val TRANSPORT_USB = 8
private const val TRANSPORT_SATELLITE = 10

/** The last field of the transport bitset's bytes. */
private const val TRANSPORTS_OFFSET = 32

/** An Android VPN over Wi-Fi, as `ANDROID_VPN_OVER_WIFI` in the Rust crate's tests decodes it. */
private val VPN_OVER_WIFI =
    byteArrayOf(
        6,
        0,
        0,
        0,
        2,
        0,
        0,
        0,
        2,
        0,
        0,
        0,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        0,
        0,
        0,
        0,
        0x11,
        0,
        0,
        0,
    )

/** The change call must answer well within the test's patience. */
private const val CHANGE_TIMEOUT_MS = 2_000L

/** Codes the wire contract allows for the connection field. */
private val CONNECTION_CODES = 0..7

@RunWith(AndroidJUnit4::class)
class NetworkPluginTest {
    @Test
    fun snapshotAndChangedSnapshotUseTheRealConnectivityService() =
        runBlocking {
            withContext(Dispatchers.Main) {
                withPlugin { plugin ->
                    validate(plugin.invoke(SNAPSHOT, ByteBuffer.allocate(0)))
                    val changed = awaitChange(plugin)
                    validate(changed)
                    assertFalse(changed.contentEquals(ByteArray(SNAPSHOT_BYTES)))
                }
            }
        }

    @Test
    fun cancellationReleasesObserverBeforeImmediateReplacementAndClose() =
        runBlocking {
            withContext(Dispatchers.Main) {
                withPlugin { plugin ->
                    val current = plugin.invoke(SNAPSHOT, ByteBuffer.allocate(0))
                    val first = startChange(plugin, current)
                    first.cancelAndJoin()
                    // A cancelled request must not leave a stale Busy waiter.
                    val replacement = startChange(plugin, current)
                    assertFalse(replacement.isCancelled)
                    plugin.close()
                    replacement.cancelAndJoin()
                    assertTrue(replacement.isCompleted)
                }
            }
        }

    @Test
    fun transportBitsMatchTheRustWire() {
        assertEquals(0x11, NetworkTransports.bits(listOf(TRANSPORT_WIFI, TRANSPORT_VPN), PRIMARY_VPN))
        assertEquals(
            0x11,
            ByteBuffer.wrap(VPN_OVER_WIFI, TRANSPORTS_OFFSET, 4).order(ByteOrder.LITTLE_ENDIAN).int,
        )
        assertEquals(NetworkTransports.WIFI, NetworkTransports.bits(listOf(TRANSPORT_WIFI_AWARE), PRIMARY_OTHER) and NetworkTransports.WIFI)
        assertEquals(
            NetworkTransports.CELLULAR or NetworkTransports.SATELLITE,
            NetworkTransports.bits(listOf(TRANSPORT_CELLULAR, TRANSPORT_SATELLITE), PRIMARY_CELLULAR),
        )
        assertEquals(
            NetworkTransports.ETHERNET or NetworkTransports.BLUETOOTH or NetworkTransports.OTHER,
            NetworkTransports.bits(listOf(TRANSPORT_ETHERNET, TRANSPORT_BLUETOOTH, TRANSPORT_USB), PRIMARY_OTHER),
        )
        assertEquals(NetworkTransports.OTHER, NetworkTransports.bits(emptyList(), PRIMARY_OTHER))
        assertEquals(NetworkTransports.WIFI, NetworkTransports.bits(emptyList(), PRIMARY_WIFI))
        assertEquals(0, NetworkTransports.bits(emptyList(), PRIMARY_NONE))
        assertEquals(NetworkTransports.MASK, NetworkTransports.bits(NetworkTransports.CODES, PRIMARY_OTHER))
    }

    @Test
    fun previousReadingsWithUnknownTransportBitsOrTheOldSizeAreRefused() =
        runBlocking {
            withContext(Dispatchers.Main) {
                withPlugin { plugin ->
                    val unknownBit = VPN_OVER_WIFI.copyOf()
                    unknownBit[TRANSPORTS_OFFSET] = 0x80.toByte()
                    for (payload in listOf(unknownBit, VPN_OVER_WIFI.copyOf(LEGACY_SNAPSHOT_BYTES))) {
                        val error =
                            runCatching { plugin.invoke(WAIT_FOR_CHANGE, ByteBuffer.wrap(payload)) }
                                .exceptionOrNull()
                        assertEquals(
                            PluginException.INVALID_ARGUMENT,
                            (error as PluginException).code,
                        )
                    }
                }
            }
        }

    /** Runs the block against a plugin bound to the target context, closing it after. */
    private suspend fun withPlugin(block: suspend (NetworkPlugin) -> Unit) {
        val plugin = NetworkPlugin(InstrumentationRegistry.getInstrumentation().targetContext)
        try {
            block(plugin)
        } finally {
            plugin.close()
        }
    }

    /** Waits for the next reading the real connectivity service reports. */
    private suspend fun awaitChange(plugin: NetworkPlugin): ByteArray =
        withTimeout(CHANGE_TIMEOUT_MS) {
            plugin.invoke(WAIT_FOR_CHANGE, ByteBuffer.allocate(SNAPSHOT_BYTES))
        }

    /** Starts one change request undispatched, so a cancel races its registration. */
    private fun CoroutineScope.startChange(
        plugin: NetworkPlugin,
        previous: ByteArray,
    ): Deferred<ByteArray> =
        async(start = CoroutineStart.UNDISPATCHED) {
            plugin.invoke(WAIT_FOR_CHANGE, ByteBuffer.wrap(previous))
        }

    private fun validate(bytes: ByteArray) {
        assertEquals(SNAPSHOT_BYTES, bytes.size)
        val input = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        assertTrue(input.int in CONNECTION_CODES)
        repeat(4) { assertTrue(input.int in 0..2) }
        input.int
        input.int
        assertTrue(input.int in 0..4)
        val transports = input.int
        assertEquals(0, transports and NetworkTransports.MASK.inv())
        if (bytes[0].toInt() == PRIMARY_NONE) assertEquals(0, transports)
    }
}
