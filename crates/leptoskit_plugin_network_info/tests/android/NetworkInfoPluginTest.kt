package dev.leptoskit.plugins.networkinfo

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** The snapshot call, which takes no arguments. */
private const val SNAPSHOT = 0

/** The fully populated snapshot the Rust and Swift suites assert too. */
private val GOLDEN_HEX =
    "7f000000" + "04000000" + "686f6d65" + "00".repeat(28) +
        "123456789abc" + "0000" + "c0a80114" + "20010db8000000000000000000000001" +
        "ffffff00" + "c0a801ff" + "c0a80101"

/** Offsets of the BSSID and of the first address field. */
private const val BSSID_OFFSET = 40
private const val IPV4_OFFSET = 48

/** Every presence bit the wire defines. */
private const val PRESENCE_MASK = 0x7F

private fun hex(bytes: ByteArray): String = bytes.joinToString("") { "%02x".format(it) }

private fun bytes(vararg values: Int): ByteArray = ByteArray(values.size) { values[it].toByte() }

@RunWith(AndroidJUnit4::class)
class NetworkInfoPluginTest {
    @Test
    fun aFullReadingMatchesTheSharedGoldenBytes() {
        val reading =
            NetworkInfoWire.encode(
                NetworkInfoWire.Fields(
                    ssid = NetworkInfoWire.ssid("\"home\""),
                    bssid = NetworkInfoWire.bssid("12:34:56:78:9a:bc"),
                    ipv4 = bytes(192, 168, 1, 20),
                    ipv6 = bytes(0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1),
                    subnetMask = NetworkInfoWire.mask(24),
                    broadcast = NetworkInfoWire.broadcast(bytes(192, 168, 1, 20), 24),
                    gateway = bytes(192, 168, 1, 1),
                ),
            )
        assertEquals(GOLDEN_HEX, hex(reading))
        assertArrayEquals(ByteArray(NetworkInfoWire.SNAPSHOT_BYTES), NetworkInfoWire.encode(NetworkInfoWire.Fields()))
    }

    @Test
    fun ssidsAreDecodedFromQuotesOrHexAndPlaceholdersAreWithheld() {
        assertArrayEquals("home".toByteArray(), NetworkInfoWire.ssid("\"home\""))
        assertArrayEquals("café".toByteArray(), NetworkInfoWire.ssid("\"café\""))
        assertArrayEquals(bytes(0xFF, 0x00, 0x41), NetworkInfoWire.ssid("ff0041"))
        assertEquals(32, NetworkInfoWire.ssid("\"" + "x".repeat(32) + "\"")?.size)

        for (withheld in listOf(null, "", "<unknown ssid>", "\"\"", "\"" + "x".repeat(33) + "\"", "0x", "abc", "zz")) {
            assertNull(withheld, NetworkInfoWire.ssid(withheld))
        }

        val raw = NetworkInfoWire.encode(NetworkInfoWire.Fields(ssid = NetworkInfoWire.ssid("ff0041")))
        assertEquals("0100000003000000ff0041", hex(raw.copyOf(11)))
    }

    @Test
    fun bssidsRefuseTheRedactedZeroAndGroupAddresses() {
        assertArrayEquals(bytes(0xA4, 0x2B, 0xB0, 3, 4, 5), NetworkInfoWire.bssid("A4:2B:b0:3:4:5"))

        for (withheld in listOf(
            null,
            "",
            "02:00:00:00:00:00",
            "00:00:00:00:00:00",
            "ff:ff:ff:ff:ff:ff",
            "01:00:5e:00:00:01",
            "12:34:56:78:9a",
            "12:34:56:78:9a:bc:de",
            "123:4:5:6:7:8",
            "g1:00:00:00:00:01",
            "12::56:78:9a:bc",
        )) {
            assertNull(withheld, NetworkInfoWire.bssid(withheld))
        }
    }

    @Test
    fun masksAndBroadcastsFollowThePrefix() {
        assertEquals("ffffff00", hex(NetworkInfoWire.mask(24)!!))
        assertEquals("80000000", hex(NetworkInfoWire.mask(1)!!))
        assertEquals("ffffffff", hex(NetworkInfoWire.mask(32)!!))
        assertNull(NetworkInfoWire.mask(0))
        assertNull(NetworkInfoWire.mask(33))
        assertEquals("0affffff", hex(NetworkInfoWire.broadcast(bytes(10, 1, 2, 3), 8)!!))
        assertEquals("c0a80103", hex(NetworkInfoWire.broadcast(bytes(192, 168, 1, 2), 30)!!))
        assertNull(NetworkInfoWire.broadcast(bytes(192, 168, 1, 2), 31))
        assertNull(NetworkInfoWire.broadcast(bytes(192, 168, 1, 2), 32))
        assertNull(NetworkInfoWire.broadcast(bytes(192, 168, 1), 24))
    }

    @Test
    fun theRealServicesAnswerAWellFormedReadingWithoutPlaceholders() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = NetworkInfoPlugin(InstrumentationRegistry.getInstrumentation().targetContext)
                val reading = plugin.invoke(SNAPSHOT, ByteBuffer.allocate(0))
                assertEquals(NetworkInfoWire.SNAPSHOT_BYTES, reading.size)
                val input = ByteBuffer.wrap(reading).order(ByteOrder.LITTLE_ENDIAN)
                val presence = input.int
                val ssidLength = input.int
                assertEquals(0, presence and PRESENCE_MASK.inv())
                assertTrue(ssidLength in 0..NetworkInfoWire.MAX_SSID_BYTES)

                if (presence and 1 == 0) assertEquals(0, ssidLength)

                // A withheld BSSID is all zero, never Android's 02:00:00:00:00:00 placeholder.
                val bssid = reading.copyOfRange(BSSID_OFFSET, BSSID_OFFSET + 6)
                val spelled = hex(bssid).chunked(2).joinToString(":")
                assertTrue(spelled, bssid.all { it.toInt() == 0 } || NetworkInfoWire.bssid(spelled) != null)
                assertTrue(reading.copyOfRange(BSSID_OFFSET + 6, IPV4_OFFSET).all { it.toInt() == 0 })

                expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(SNAPSHOT, ByteBuffer.wrap(byteArrayOf(0))) }
                expectError(PluginException.UNAVAILABLE) { plugin.invoke(1, ByteBuffer.allocate(0)) }
                plugin.close()
                expectError(PluginException.CANCELLED) { plugin.invoke(SNAPSHOT, ByteBuffer.allocate(0)) }
            }
        }

    private suspend fun expectError(
        code: Int,
        call: suspend () -> ByteArray,
    ) {
        val error = runCatching { call() }.exceptionOrNull()
        assertEquals(code, (error as PluginException).code)
    }
}
