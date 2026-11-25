package dev.leptoskit.plugins.background

import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Validates the wire codec and status shape without starting a real service. */
class BackgroundPluginTest {
    @Test
    fun statusReportsStrictShapeAndStartRejectsMalformedPayloads() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val plugin = withContext(Dispatchers.Main) { BackgroundPlugin(context) }
            try {
                val status =
                    withContext(Dispatchers.Main) {
                        plugin.invoke(BackgroundPlugin.STATUS, ByteBuffer.allocate(0))
                    }
                assertEquals(12, status.size)
                val running = ByteBuffer.wrap(status).order(ByteOrder.LITTLE_ENDIAN).int
                assertEquals(0, running)

                val malformed = start(ByteArray(2))
                try {
                    withContext(Dispatchers.Main) {
                        plugin.invoke(BackgroundPlugin.START, ByteBuffer.wrap(malformed))
                    }
                    fail("Truncated start payload accepted")
                } catch (error: PluginException) {
                    assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                }

                try {
                    withContext(Dispatchers.Main) {
                        plugin.invoke(BackgroundPlugin.START, ByteBuffer.wrap(start(valid())))
                    }
                    fail("Start payload with a zero interval accepted")
                } catch (error: PluginException) {
                    assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                }
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
            }
        }

    @Test
    fun emptyPayloadsAreRequiredForStopAndStatus() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val plugin = withContext(Dispatchers.Main) { BackgroundPlugin(context) }
            try {
                for (method in intArrayOf(BackgroundPlugin.STOP, BackgroundPlugin.STATUS)) {
                    try {
                        withContext(Dispatchers.Main) {
                            plugin.invoke(method, ByteBuffer.wrap(byteArrayOf(1)))
                        }
                        fail("Method $method accepted a nonempty payload")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                    }
                }
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
            }
        }

    @Test
    fun foregroundServiceTypeMappingMatchesAndroidServiceInfo() {
        assertEquals(0, mapForegroundServiceTypes(0, 28))
        assertEquals(0, mapForegroundServiceTypes(0, 29))
        assertEquals(0, mapForegroundServiceTypes(0, 34))

        assertEquals(ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC, mapForegroundServiceTypes(CRATE_TYPE_DATA_SYNC, 34))
        assertEquals(ServiceInfo.FOREGROUND_SERVICE_TYPE_HEALTH, mapForegroundServiceTypes(CRATE_TYPE_HEALTH, 34))
        assertEquals(
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
            mapForegroundServiceTypes(CRATE_TYPE_MICROPHONE, 34),
        )
        assertEquals(ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION, mapForegroundServiceTypes(CRATE_TYPE_LOCATION, 34))
        assertEquals(
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            mapForegroundServiceTypes(CRATE_TYPE_CONNECTED_DEVICE, 34),
        )

        // Types drop on OS versions before Android added their constants.
        assertEquals(0, mapForegroundServiceTypes(ALL_CRATE_TYPES, 28))
        assertEquals(
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC or
                ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION or
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            mapForegroundServiceTypes(ALL_CRATE_TYPES, 29),
        )
        assertEquals(0, mapForegroundServiceTypes(CRATE_TYPE_MICROPHONE, 29))
        assertEquals(ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE, mapForegroundServiceTypes(CRATE_TYPE_MICROPHONE, 30))
        assertEquals(0, mapForegroundServiceTypes(CRATE_TYPE_HEALTH, 33))
        assertEquals(
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC or
                ServiceInfo.FOREGROUND_SERVICE_TYPE_HEALTH or
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE or
                ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION or
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            mapForegroundServiceTypes(ALL_CRATE_TYPES, 34),
        )
    }

    @Test
    fun configPersistsThroughCodecSharedPreferencesAndIntentExtras() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val config =
            BackgroundConfig(
                title = "Quant is running ✓",
                text = "Syncing background work",
                types = CRATE_TYPE_DATA_SYNC or CRATE_TYPE_HEALTH,
                intervalSecs = 45,
            )

        assertEquals(config, BackgroundConfigCodec.decode(BackgroundConfigCodec.encode(config)))
        assertNull(BackgroundConfigCodec.decode(emptyMap<String, Any>()))

        try {
            BackgroundConfigStore.clear(context)
            assertNull(BackgroundConfigStore.load(context))

            BackgroundConfigStore.save(context, config)
            assertEquals(config, BackgroundConfigStore.load(context))

            val intent = BackgroundService.startIntent(context, config)
            assertEquals(StartIntent.WithConfig(config), BackgroundService.classifyIntent(intent))
            assertEquals(StartIntent.Bare, BackgroundService.classifyIntent(Intent(context, BackgroundService::class.java)))
            assertEquals(StartIntent.Null, BackgroundService.classifyIntent(null))

            BackgroundConfigStore.clear(context)
            assertNull(BackgroundConfigStore.load(context))
        } finally {
            BackgroundConfigStore.clear(context)
        }
    }

    @Test
    fun restartDecisionsPreserveConfigurationAndAvoidZeroTypesOnApi34() {
        val persisted =
            BackgroundConfig(
                title = "Persisted",
                text = "Working",
                types = CRATE_TYPE_DATA_SYNC or CRATE_TYPE_MICROPHONE,
                intervalSecs = 30,
            )
        val explicit =
            BackgroundConfig(
                title = "Explicit",
                text = "From intent",
                types = CRATE_TYPE_HEALTH,
                intervalSecs = 15,
            )

        assertEquals(
            StartCommandDecision.StartForeground(explicit, ServiceInfo.FOREGROUND_SERVICE_TYPE_HEALTH),
            decideStartCommand(StartIntent.WithConfig(explicit), persisted, 34),
        )
        assertEquals(
            StartCommandDecision.StartForeground(
                persisted,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
            ),
            decideStartCommand(StartIntent.Bare, persisted, 34),
        )
        assertEquals(StartCommandDecision.Stop, decideStartCommand(StartIntent.Bare, null, 34))
        assertEquals(StartCommandDecision.Stop, decideStartCommand(StartIntent.Null, null, 34))
        assertEquals(
            StartCommandDecision.StartForeground(
                persisted,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
            ),
            decideStartCommand(StartIntent.Null, persisted, 34),
        )
        assertEquals(
            StartCommandDecision.Stop,
            decideStartCommand(StartIntent.WithConfig(persisted.copy(types = 0)), null, 34),
        )

        assertEquals(WorkerDecision.WakeOnly, decideWorkerAction(true, persisted, 34))
        assertEquals(WorkerDecision.StartService(persisted), decideWorkerAction(false, persisted, 34))
        assertEquals(WorkerDecision.CancelWork, decideWorkerAction(false, null, 34))
        assertEquals(WorkerDecision.CancelWork, decideWorkerAction(false, persisted.copy(types = 0), 34))

        assertEquals(BootDecision.DoNothing, decideBootAction(false, persisted, 35))
        assertEquals(BootDecision.DoNothing, decideBootAction(true, null, 35))
        assertEquals(BootDecision.StartService(persisted), decideBootAction(true, persisted, 35))
    }

    @Test
    fun startRejectsUnknownServiceTypesAndZeroTypesOnApi34() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val plugin = withContext(Dispatchers.Main) { BackgroundPlugin(context) }
            try {
                try {
                    withContext(Dispatchers.Main) {
                        plugin.invoke(
                            BackgroundPlugin.START,
                            ByteBuffer.wrap(encodeStartPayload("Running", "", 1 shl 5, 60)),
                        )
                    }
                    fail("Start payload with unknown type bit accepted")
                } catch (error: PluginException) {
                    assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                }

                if (Build.VERSION.SDK_INT >= 34) {
                    try {
                        withContext(Dispatchers.Main) {
                            plugin.invoke(
                                BackgroundPlugin.START,
                                ByteBuffer.wrap(encodeStartPayload("Running", "", 0, 60)),
                            )
                        }
                        fail("Start payload with zero service type accepted on API 34+")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                    }
                }
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
            }
        }

    private fun encodeStartPayload(
        title: String,
        text: String,
        types: Int,
        interval: Int,
    ): ByteArray {
        val titleBytes = title.toByteArray(Charsets.UTF_8)
        val textBytes = text.toByteArray(Charsets.UTF_8)
        val buffer =
            ByteBuffer
                .allocate(16 + titleBytes.size + textBytes.size)
                .order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(titleBytes.size).put(titleBytes)
        buffer.putInt(textBytes.size).put(textBytes)
        buffer.putInt(types)
        buffer.putInt(interval)

        return buffer.array()
    }

    private fun valid(): ByteArray {
        val title = "Running".toByteArray(Charsets.UTF_8)
        val text = ByteArray(0)
        val buffer = ByteBuffer.allocate(8 + title.size + text.size).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(title.size).put(title)
        buffer.putInt(text.size).put(text)
        buffer.putInt(0)
        buffer.putInt(60)

        return buffer.array()
    }

    private fun start(payload: ByteArray): ByteArray {
        val title = "Running".toByteArray(Charsets.UTF_8)
        val buffer = ByteBuffer.allocate(8 + title.size + payload.size).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(title.size).put(title)
        buffer.putInt(payload.size).put(payload)

        return buffer.array()
    }
}
