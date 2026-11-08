package dev.leptoskit.plugins.devicekeys

import android.content.Context
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** No test invokes a system biometric prompt or creates a real signing key. */
class DeviceKeysPluginTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun availabilityReportsAWellFormedSnapshotWithoutPrompting() =
        withPlugin(context) { plugin ->
            val report =
                ByteBuffer
                    .wrap(plugin.invoke(AVAILABLE, ByteBuffer.allocate(0)))
                    .order(ByteOrder.LITTLE_ENDIAN)

            assertEquals(16, report.capacity())
            assertEquals(WIRE_VERSION, report.getInt(0))
            val hardware = report.getInt(4)
            val biometry = report.getInt(8)
            val enrolled = report.getInt(12)
            assertTrue("hardware out of range: $hardware", hardware in HARDWARE_SECURE_ENCLAVE..HARDWARE_NONE)
            assertTrue("biometry out of range: $biometry", biometry in BIOMETRY_TOUCH_ID..BIOMETRY_NONE)
            assertTrue("enrolled out of range: $enrolled", enrolled == 0 || enrolled == 1)
        }

    @Test
    fun malformedRequestsAreRejectedBeforeAnyKeyWork() =
        runBlocking {
            val generate = generateRequest()
            assertEquals("payout.v2", GenerateRequest.decode(generate).alias)

            generate.putInt(POLICY_OFFSET, UNKNOWN_POLICY)
            assertRejects(PluginException.INVALID_ARGUMENT, "Unknown policy accepted") {
                GenerateRequest.decode(generate)
            }

            val sign = signRequest()
            assertEquals("Approve transfer", SignRequest.decode(sign).reason)

            sign.put(ALIAS_OFFSET, INVALID_UTF8)
            assertRejects(PluginException.INVALID_ARGUMENT, "Invalid UTF-8 accepted") {
                SignRequest.decode(sign)
            }

            assertRejects(PluginException.INVALID_ARGUMENT, "Unknown method accepted") {
                decodeAlias(ByteBuffer.wrap(deviceKeysFields(WIRE_VERSION)))
            }
        }

    @Test
    fun closedPluginRejectsNewWork() =
        withPlugin(context) { plugin ->
            plugin.close()

            assertRejects(PluginException.CANCELLED, "Closed plugin accepted work") {
                plugin.invoke(AVAILABLE, ByteBuffer.allocate(0))
            }
        }

    /** Creates the plugin on the UI thread, runs the body, then closes it. */
    private fun withPlugin(
        context: Context,
        body: suspend CoroutineScope.(DeviceKeysPlugin) -> Unit,
    ) = runBlocking {
        withContext(Dispatchers.Main) {
            val plugin = DeviceKeysPlugin(context)

            try {
                body(plugin)
            } finally {
                plugin.close()
            }
        }
    }

    /** Expects the call to fail with the given plugin code. */
    private suspend fun assertRejects(
        code: Int,
        message: String,
        call: suspend () -> Unit,
    ) {
        try {
            call()
            fail(message)
        } catch (error: PluginException) {
            assertEquals(code, error.code)
        }
    }

    /** A minimal generate request for the biometry policy. */
    private fun generateRequest(): ByteBuffer {
        val alias = "payout.v2".toByteArray()

        return ByteBuffer
            .allocate(8 + alias.size)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(WIRE_VERSION)
            .putInt(POLICY_BIOMETRY)
            .putInt(alias.size)
            .put(alias)
            .apply { flip() }
    }

    /** A minimal sign request. */
    private fun signRequest(): ByteBuffer {
        val alias = "payout.v2".toByteArray()
        val reason = "Approve transfer".toByteArray()
        val message = byteArrayOf(1, 2, 3)

        return ByteBuffer
            .allocate(4 + 4 + alias.size + 4 + message.size + 4 + reason.size)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(WIRE_VERSION)
            .putInt(alias.size)
            .put(alias)
            .putInt(message.size)
            .put(message)
            .putInt(reason.size)
            .put(reason)
            .apply { flip() }
    }

    private companion object {
        const val AVAILABLE = 4

        /** Where the policy and the alias's first byte sit in an encoded request. */
        const val POLICY_OFFSET = 4
        const val ALIAS_OFFSET = 8

        /** Values the decoder must reject. */
        const val UNKNOWN_POLICY = 2
        val INVALID_UTF8 = 255.toByte()
    }
}
