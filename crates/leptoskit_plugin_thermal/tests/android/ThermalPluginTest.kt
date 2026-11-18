package dev.leptoskit.plugins.thermal

import android.os.Build
import android.os.PowerManager
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.yield
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Wire fixtures match the Rust crate's golden bytes; the device's own status is read, never forced. */
class ThermalPluginTest {
    @Test
    fun everyStatusFoldsLikeTheRustCrate() {
        val expected =
            intArrayOf(STATE_NOMINAL, STATE_FAIR, STATE_FAIR, STATE_SERIOUS, STATE_CRITICAL, STATE_CRITICAL, STATE_CRITICAL)

        for (status in 0..PowerManager.THERMAL_STATUS_SHUTDOWN) {
            assertEquals(expected[status], stateFor(status))
        }

        // Rust's `Snapshot::from_android(AndroidStatus::Moderate)`.
        assertArrayEquals(byteArrayOf(2, 0, 0, 0, 2, 0, 0, 0), encodeSnapshot(PowerManager.THERMAL_STATUS_MODERATE))
        // An unreadable or future status is unknown in both fields.
        assertArrayEquals(byteArrayOf(0, 0, 0, 0, -1, -1, -1, -1), encodeSnapshot(NO_STATUS))
        assertArrayEquals(byteArrayOf(0, 0, 0, 0, -1, -1, -1, -1), encodeSnapshot(LATER_STATUS))
    }

    @Test
    fun validationRefusesWhatRustRefuses() {
        validateSnapshot(snapshot(STATE_SERIOUS, NO_STATUS))
        validateSnapshot(snapshot(STATE_SERIOUS, PowerManager.THERMAL_STATUS_SEVERE))

        for ((state, status) in listOf(
            STATE_CRITICAL + 1 to NO_STATUS,
            STATE_NOMINAL to PowerManager.THERMAL_STATUS_SEVERE,
            STATE_CRITICAL to LATER_STATUS,
        )) {
            assertInvalid { validateSnapshot(snapshot(state, status)) }
        }

        assertInvalid { validateSnapshot(ByteBuffer.allocate(SNAPSHOT_BYTES - 1)) }
    }

    @Test
    fun liveSnapshotWaitAndClose() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = ThermalPlugin(InstrumentationRegistry.getInstrumentation().targetContext)
                val current = plugin.invoke(0, ByteBuffer.allocateDirect(0))
                assertEquals(SNAPSHOT_BYTES, current.size)

                if (Build.VERSION.SDK_INT >= THERMAL_API) waitAndClose(plugin, current) else plugin.close()

                assertCode(PluginException.CANCELLED) { plugin.invoke(0, ByteBuffer.allocateDirect(0)) }
            }
        }

    /** One waiter answers at once, a second is busy, and close ends the one still waiting. */
    private suspend fun waitAndClose(
        plugin: ThermalPlugin,
        current: ByteArray,
    ) = coroutineScope {
        // A caller whose last reading was unknown hears the current status at once.
        assertArrayEquals(current, plugin.invoke(1, direct(encodeSnapshot(NO_STATUS))))

        val pending = async { plugin.invoke(1, direct(current)) }
        yield()
        assertCode(PluginException.BUSY) { plugin.invoke(1, direct(current)) }
        plugin.close()
        runCatching { pending.await() }
    }

    private fun snapshot(
        state: Int,
        status: Int,
    ): ByteBuffer =
        ByteBuffer
            .allocate(SNAPSHOT_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(state)
            .putInt(status)
            .flip() as ByteBuffer

    private fun direct(bytes: ByteArray): ByteBuffer = ByteBuffer.allocateDirect(bytes.size).put(bytes).flip() as ByteBuffer

    private fun assertInvalid(block: () -> Unit) {
        try {
            block()
            fail("expected INVALID_ARGUMENT")
        } catch (error: PluginException) {
            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
        }
    }

    private suspend fun assertCode(
        code: Int,
        block: suspend () -> Unit,
    ) {
        try {
            block()
            fail("expected plugin error $code")
        } catch (error: PluginException) {
            assertEquals(code, error.code)
        }
    }

    private companion object {
        /** A status a newer Android might add. */
        const val LATER_STATUS = 7

        /** `PowerManager` thermal status first appears at this API level. */
        const val THERMAL_API = 29
    }
}
