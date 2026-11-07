package dev.leptoskit.plugins.device

import android.os.Build
import android.provider.Settings
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Reads the fields [WireWriter] writes, failing on anything malformed. */
private class WireReader(
    bytes: ByteArray,
) {
    private val input = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)

    fun u32(): Int = input.int

    fun u64(): Long = input.long

    fun tri(): Boolean? =
        when (val value = u32()) {
            0 -> null
            1 -> false
            2 -> true
            else -> throw AssertionError("invalid tri-state $value")
        }

    fun optionalU32(): Int? {
        val present = u32()
        val value = u32()
        assertTrue(present == 1 || (present == 0 && value == 0))
        return if (present == 1) value else null
    }

    fun optionalU64(): Long? {
        val present = u32()
        val value = u64()
        assertTrue(present == 1 || (present == 0 && value == 0L))
        return if (present == 1) value else null
    }

    fun text(): String? {
        val bytes = ByteArray(u32()).also { input.get(it) }
        return bytes.toString(Charsets.UTF_8).ifEmpty { null }
    }

    fun list(): List<String> = List(u32()) { text()!! }

    fun finish() = assertFalse("trailing bytes", input.hasRemaining())
}

@RunWith(AndroidJUnit4::class)
class DevicePluginTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun snapshotBytesMatchTheRustGolden() {
        assertEquals(ANDROID_GOLDEN, DeviceWire.snapshot(ANDROID_FIXTURE).hex())
        assertEquals(IDENTIFIERS_GOLDEN, DeviceWire.identifiers(IDENTIFIERS_FIXTURE).hex())
    }

    @Test
    fun realSnapshotReportsThisBuild() =
        runBlocking {
            val reply = DevicePlugin(context).invoke(0, ByteBuffer.allocate(0))
            val input = WireReader(reply)

            assertEquals(2, input.u32())
            assertTrue(input.u32() in 0..7)
            val simulator = input.tri()
            val totalMemory = input.u64()
            assertTrue(totalMemory > 0)
            assertTrue(input.optionalU64()!! in 0..totalMemory)
            val totalDisk = input.u64()
            assertTrue(totalDisk > 0)
            assertTrue(input.optionalU64()!! in 0..totalDisk)
            assertEquals(Runtime.getRuntime().availableProcessors(), input.u32())
            assertEquals(DeviceWire.known(Build.MANUFACTURER), input.text())
            assertEquals(DeviceWire.known(Build.BRAND), input.text())
            assertEquals(DeviceWire.known(Build.MODEL), input.text())
            assertNull("Android has no model identifier", input.text())
            assertEquals("Android", input.text())
            assertEquals(Build.VERSION.RELEASE, input.text())
            assertEquals(Build.SUPPORTED_ABIS.first(), input.text())
            assertEquals(System.getProperty("os.version"), input.text())

            assertEquals(2, input.u32())
            assertEquals(Build.VERSION.SDK_INT, input.u32())
            assertEquals(Build.VERSION.PREVIEW_SDK_INT, input.optionalU32())
            assertTrue(input.tri() != null)
            assertTrue(input.tri() != null)
            assertEquals(Build.TIME, input.optionalU64())
            val strings = List(16) { input.text() }
            assertEquals(DeviceWire.known(Build.VERSION.CODENAME), strings[1])
            assertEquals(DeviceWire.known(Build.VERSION.SECURITY_PATCH), strings[4])
            assertEquals(DeviceWire.known(Build.DEVICE), strings[7])
            assertEquals(DeviceWire.known(Build.FINGERPRINT), strings[9])
            assertEquals(DeviceWire.known(Build.ID), strings[12])
            assertEquals(DeviceWire.known(Build.TYPE), strings[15])
            assertEquals(Build.SUPPORTED_ABIS.toList(), input.list())
            assertEquals(Build.SUPPORTED_32_BIT_ABIS.toList(), input.list())
            assertEquals(Build.SUPPORTED_64_BIT_ABIS.toList(), input.list())
            val features = input.list()
            assertTrue(features.isNotEmpty())
            assertEquals(features.sorted().distinct(), features)
            assertTrue(features.all { context.packageManager.hasSystemFeature(it) })
            input.finish()

            if (Build.HARDWARE == "ranchu" || Build.HARDWARE == "goldfish") assertEquals(true, simulator)
        }

    @Test
    fun identifiersCarryOnlyTheDeviceName() =
        runBlocking {
            val input = WireReader(DevicePlugin(context).invoke(1, ByteBuffer.allocate(0)))
            val expected = Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)

            assertEquals(1, input.u32())
            assertEquals(DeviceWire.boundedIdentifier(expected), input.text())
            assertNull("no vendor identifier on Android", input.text())
            assertNull("no machine identifier on Android", input.text())
            input.finish()
        }

    @Test
    fun rejectsUnknownMethodsAndPayloads() =
        runBlocking {
            val plugin = DevicePlugin({ ANDROID_FIXTURE }, { IDENTIFIERS_FIXTURE })
            assertEquals(listOf(0, 1), plugin.methods.toList())
            assertEquals(ANDROID_GOLDEN, plugin.invoke(0, ByteBuffer.allocate(0)).hex())
            assertEquals(IDENTIFIERS_GOLDEN, plugin.invoke(1, ByteBuffer.allocate(0)).hex())
            assertCode(PluginException.UNAVAILABLE) { plugin.invoke(2, ByteBuffer.allocate(0)) }
            assertCode(PluginException.INVALID_ARGUMENT) { plugin.invoke(0, ByteBuffer.wrap(byteArrayOf(1))) }
            assertCode(PluginException.INVALID_ARGUMENT) { plugin.invoke(1, ByteBuffer.wrap(byteArrayOf(1))) }
        }

    @Test
    fun boundsTextAndLists() {
        val long = "x".repeat(DeviceWire.MAX_TEXT_BYTES + 1)
        assertCode(PluginException.TOO_LARGE) { DeviceWire.snapshot(ANDROID_FIXTURE.copy(model = long)) }
        val features = ANDROID_FIXTURE.build.copy(systemFeatures = List(DeviceWire.MAX_FEATURES + 1) { "f$it" })
        assertCode(PluginException.TOO_LARGE) { DeviceWire.snapshot(ANDROID_FIXTURE.copy(build = features)) }
        val empty = ANDROID_FIXTURE.build.copy(supportedAbis = listOf(""))
        assertCode(PluginException.INVALID_ARGUMENT) { DeviceWire.snapshot(ANDROID_FIXTURE.copy(build = empty)) }

        assertEquals(listOf("a", "b"), DeviceWire.shortList(listOf("a", "", null, "unknown", "b")))
        assertEquals(DeviceWire.MAX_SHORT_LIST, DeviceWire.shortList(List(40) { "abi$it" }).size)
        assertEquals(
            listOf("a.feature", "b.feature"),
            DeviceWire.features(listOf("b.feature", "a.feature", "b.feature", " ", "x".repeat(300))),
        )
        assertEquals(DeviceWire.MAX_FEATURES, DeviceWire.features(List(2000) { "f$it" }).size)
        assertNull(DeviceWire.boundedIdentifier("n".repeat(DeviceWire.MAX_IDENTIFIER_BYTES + 1)))
        assertNull(DeviceWire.boundedIdentifier(" "))
        assertEquals("Pixel", DeviceWire.boundedIdentifier("Pixel"))
    }

    private fun assertCode(
        code: Int,
        block: suspend () -> Unit,
    ) {
        val error = assertThrows(PluginException::class.java) { runBlocking { block() } }
        assertEquals(code, error.code)
    }
}
