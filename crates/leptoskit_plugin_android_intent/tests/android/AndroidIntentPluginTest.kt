package dev.leptoskit.plugins.androidintent

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Exercises the plugin against the real PackageManager and ActivityManager.
 * The only external activity it opens is system Settings, and it returns home
 * afterwards. Everything else targets this app's own components.
 */
class AndroidIntentPluginTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext

    private fun plugin() = AndroidIntentPlugin(context)

    private fun call(
        plugin: AndroidIntentPlugin,
        method: Int,
        payload: ByteArray,
    ): ByteArray = runBlocking { withContext(Dispatchers.Main) { plugin.invoke(method, ByteBuffer.wrap(payload)) } }

    private fun code(
        method: Int,
        payload: ByteArray,
    ): Int = failure { call(plugin(), method, payload) }

    private fun launcherClass(): String =
        checkNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName)?.component).className

    @Test
    fun wireRulesMatchTheRustCrate() {
        androidIntentWireChecks()
    }

    @Test
    fun resolvesThisAppsOwnActivityWithItsLabel() {
        val plugin = plugin()
        val own = intentBytes(action = null, packageName = context.packageName, component = launcherClass())

        assertArrayEquals(byteArrayOf(1, 0, 0, 0), call(plugin, CAN_RESOLVE_ACTIVITY, own))
        val reply = ByteBuffer.wrap(call(plugin, RESOLVED_ACTIVITY, own)).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(1, reply.int)
        assertEquals(context.packageName, string(reply))
        assertEquals(launcherClass(), string(reply))
        assertEquals(context.applicationInfo.loadLabel(context.packageManager).toString(), string(reply))
        assertFalse(reply.hasRemaining())

        val missing = intentBytes(action = "dev.leptoskit.test.NO_SUCH_ACTION", packageName = context.packageName)
        assertArrayEquals(byteArrayOf(0, 0, 0, 0), call(plugin, CAN_RESOLVE_ACTIVITY, missing))
        assertArrayEquals(byteArrayOf(0, 0, 0, 0), call(plugin, RESOLVED_ACTIVITY, missing))
    }

    @Test
    fun resolvingSettingsAgreesWithThePackageManager() {
        val expected =
            @Suppress("DEPRECATION")
            context.packageManager.resolveActivity(Intent(Settings.ACTION_SETTINGS), PackageManager.MATCH_DEFAULT_ONLY)
        val settings = intentBytes(action = Settings.ACTION_SETTINGS)
        val resolved = call(plugin(), CAN_RESOLVE_ACTIVITY, settings)

        assertArrayEquals(byteArrayOf(if (expected != null) 1 else 0, 0, 0, 0), resolved)

        val reply = ByteBuffer.wrap(call(plugin(), RESOLVED_ACTIVITY, settings)).order(ByteOrder.LITTLE_ENDIAN)

        if (expected == null) {
            assertEquals(0, reply.int)
        } else {
            assertEquals(1, reply.int)
            assertEquals(expected.activityInfo.packageName, string(reply))
            assertEquals(expected.activityInfo.name, string(reply))
        }
    }

    @Test
    fun launchesSystemSettingsThenReturnsHome() {
        val device = UiDevice.getInstance(instrumentation)

        try {
            assertArrayEquals(byteArrayOf(), call(plugin(), LAUNCH, intentBytes(action = Settings.ACTION_SETTINGS)))
            assertTrue("Settings did not open", device.wait(Until.hasObject(By.pkg(SETTINGS_PACKAGE)), WAIT_MILLIS))
        } finally {
            device.pressHome()
        }
    }

    @Test
    fun missingActivitiesAndServicesAreUnavailable() {
        val missing = intentBytes(action = "dev.leptoskit.test.NO_SUCH_ACTION", packageName = context.packageName)
        assertEquals(PluginException.UNAVAILABLE, code(LAUNCH, missing))

        val absent = intentBytes(action = null, packageName = context.packageName, component = ".NoSuchActivity")
        assertEquals(PluginException.UNAVAILABLE, code(LAUNCH, absent))

        val service = intentBytes(action = null, packageName = context.packageName, component = ".NoSuchService")
        assertEquals(PluginException.UNAVAILABLE, code(SEND_SERVICE, service))
        assertEquals(PluginException.INVALID_ARGUMENT, code(SEND_SERVICE, intentBytes()))
    }

    @Test
    fun policyRefusalsHappenBeforeAndroidSeesTheIntent() {
        val granted = intentBytes(action = Settings.ACTION_SETTINGS, flags = 0x01)
        assertEquals(PluginException.PERMISSION_DENIED, code(LAUNCH, granted))
        assertEquals(PluginException.PERMISSION_DENIED, code(SEND_BROADCAST, granted))
        assertEquals(PluginException.PERMISSION_DENIED, code(LAUNCH, intentBytes(data = "file:///sdcard/x")))
        assertEquals(PluginException.INVALID_ARGUMENT, code(LAUNCH, byteArrayOf()))
        assertEquals(PluginException.UNAVAILABLE, code(UNKNOWN_METHOD, byteArrayOf()))

        val chooser = Wire().string("a\nb").raw(intentBytes()).bytes()
        assertEquals(PluginException.INVALID_ARGUMENT, code(LAUNCH_CHOOSER, chooser))
    }

    @Test
    fun intentUrisAreHandledLikeABrowserWould() {
        val cases =
            listOf(
                "intent:#Intent;component=com.android.settings/.Settings;end" to PluginException.PERMISSION_DENIED,
                "intent:#Intent;action=a.B;SEL;action=android.settings.SETTINGS;end" to PluginException.PERMISSION_DENIED,
                "intent:#Intent;action=android.settings.SETTINGS;launchFlags=0x1;end" to PluginException.PERMISSION_DENIED,
                "intent://x/#Intent;scheme=file;action=android.intent.action.VIEW;end" to PluginException.PERMISSION_DENIED,
                "intent:#Intent;action=a.B;launchFlags=zz;end" to PluginException.INVALID_ARGUMENT,
                // CATEGORY_BROWSABLE is added, so Settings' own entry point is not reachable this way.
                "intent:#Intent;action=android.settings.SETTINGS;end" to PluginException.UNAVAILABLE,
                "intent:#Intent;action=dev.leptoskit.test.NO_SUCH_ACTION;end" to PluginException.UNAVAILABLE,
                "https://example.com" to PluginException.INVALID_ARGUMENT,
            )

        for ((uri, expected) in cases) {
            assertEquals(uri, expected, code(PARSE_AND_LAUNCH, uri.toByteArray(Charsets.UTF_8)))
        }
    }

    @Test
    fun broadcastsDeliverEveryExtraTypeToThisAppsReceiver() {
        val action = "${context.packageName}.ANDROID_INTENT_TEST"
        val received = CountDownLatch(1)
        var extras: Bundle? = null
        val receiver =
            object : BroadcastReceiver() {
                override fun onReceive(
                    context: Context,
                    intent: Intent,
                ) {
                    extras = intent.extras
                    received.countDown()
                }
            }
        val filter = IntentFilter(action)

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            context.registerReceiver(receiver, filter)
        }

        try {
            val payload = intentBytes(action = action, packageName = context.packageName, extras = everyExtra())
            assertArrayEquals(byteArrayOf(), call(plugin(), SEND_BROADCAST, payload))
            assertTrue("broadcast not received", received.await(WAIT_MILLIS, TimeUnit.MILLISECONDS))
        } finally {
            context.unregisterReceiver(receiver)
        }

        checkEveryExtra(checkNotNull(extras))
    }

    @Test
    fun closedPluginsCancel() {
        val plugin = plugin()
        plugin.close()
        assertEquals(PluginException.CANCELLED, failure { call(plugin, LAUNCH, intentBytes()) })
    }

    private fun everyExtra(): ByteArray =
        bundleBytes(
            listOf(
                Entry("bool", 1, word(1)),
                Entry("int", 2, word(-7)),
                Entry("long", 3, Wire().i64(1L shl 40).bytes()),
                Entry("double", 4, Wire().f64(2.5).bytes()),
                Entry("string", 5, Wire().string("héllo").bytes()),
                Entry("bundle", 6, bundleBytes(listOf(Entry("inner", 2, word(1))))),
                Entry("bytes", 7, Wire().string(byteArrayOf(0, -1)).bytes()),
                Entry("bools", 8, word(2) + word(1) + word(0)),
                Entry("ints", 9, word(2) + word(1) + word(-1)),
                Entry("longs", 10, word(1) + Wire().i64(Long.MAX_VALUE).bytes()),
                Entry("doubles", 11, word(1) + Wire().f64(-0.5).bytes()),
                Entry("strings", 12, word(2) + Wire().string("a").string("β").bytes()),
                Entry("int_list", 13, word(1) + word(3)),
                Entry("string_list", 14, word(1) + Wire().string("x").bytes()),
            ),
        )

    private fun checkEveryExtra(extras: Bundle) {
        assertTrue(extras.getBoolean("bool"))
        assertEquals(-7, extras.getInt("int"))
        assertEquals(1L shl 40, extras.getLong("long"))
        assertEquals(2.5, extras.getDouble("double"), 0.0)
        assertEquals("héllo", extras.getString("string"))
        assertEquals(1, checkNotNull(extras.getBundle("bundle")).getInt("inner"))
        assertArrayEquals(byteArrayOf(0, -1), extras.getByteArray("bytes"))
        assertArrayEquals(booleanArrayOf(true, false), extras.getBooleanArray("bools"))
        assertArrayEquals(intArrayOf(1, -1), extras.getIntArray("ints"))
        assertArrayEquals(longArrayOf(Long.MAX_VALUE), extras.getLongArray("longs"))
        assertArrayEquals(doubleArrayOf(-0.5), extras.getDoubleArray("doubles"), 0.0)
        assertArrayEquals(arrayOf("a", "β"), extras.getStringArray("strings"))
        assertEquals(arrayListOf(3), extras.getIntegerArrayList("int_list"))
        assertEquals(arrayListOf("x"), extras.getStringArrayList("string_list"))
    }

    private fun string(buffer: ByteBuffer): String {
        val bytes = ByteArray(buffer.int)
        buffer.get(bytes)

        return String(bytes, Charsets.UTF_8)
    }

    private companion object {
        const val LAUNCH = 0
        const val LAUNCH_CHOOSER = 1
        const val SEND_BROADCAST = 2
        const val SEND_SERVICE = 3
        const val CAN_RESOLVE_ACTIVITY = 4
        const val RESOLVED_ACTIVITY = 5
        const val PARSE_AND_LAUNCH = 6
        const val UNKNOWN_METHOD = 99
        const val SETTINGS_PACKAGE = "com.android.settings"
        const val WAIT_MILLIS = 10_000L
    }
}
