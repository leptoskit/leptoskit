package dev.leptoskit.plugins.notifications

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.content.pm.PackageManager
import android.os.Build
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.fail
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.lang.reflect.InvocationTargetException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicInteger

/** Isolated emulator only: posts silent notifications into the generated test app. */
class NotificationsPluginTest {
    @Test
    fun localDeliveryReplacementAndCancellationAreNamespaceScoped() =
        runBlocking {
            val instrumentation = InstrumentationRegistry.getInstrumentation()
            val context = instrumentation.targetContext
            if (Build.VERSION.SDK_INT >= 33 &&
                context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
            ) {
                // This permission belongs to the disposable test app and lasts until fixture uninstall.
                instrumentation.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
            }
            val manager = context.getSystemService(NotificationManager::class.java)
            val id = "test-${System.nanoTime()}"
            val channel = "$id-channel"
            val tag = NotificationsPlugin.PREFIX + id
            val channelId = NotificationsPlugin.PREFIX + channel
            val unrelatedTag = "other-notification-provider-$id"
            val plugin = withContext(Dispatchers.Main) { NotificationsPlugin(context) }
            try {
                withContext(Dispatchers.Main) {
                    plugin.invoke(4, fields(listOf(channel, "Test channel", "Silent instrumentation"), listOf(1, 0)))
                    val configured = manager.getNotificationChannel(channelId)
                    assertNotNull(configured)
                    assertEquals(NotificationManager.IMPORTANCE_LOW, configured.importance)
                    assertNull(configured.sound)
                    try {
                        plugin.invoke(2, fields(listOf(id, "Missing channel", "", "$channel-missing")))
                        fail("Posting without an existing channel was accepted")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                    }
                    plugin.invoke(2, fields(listOf(id, "First title", "Body", channel)))
                }
                awaitNotification(manager, tag, "First title")
                withContext(Dispatchers.Main) {
                    plugin.invoke(2, fields(listOf(id, "Replacement", "Updated", channel)))
                    manager.notify(
                        unrelatedTag,
                        42,
                        Notification
                            .Builder(context, channelId)
                            .setSmallIcon(android.R.drawable.ic_dialog_info)
                            .setContentTitle("Other provider")
                            .build(),
                    )
                }
                awaitNotification(manager, tag, "Replacement")
                awaitNotification(manager, unrelatedTag, "Other provider")
                assertEquals(1, manager.activeNotifications.count { it.tag == tag })
                withContext(Dispatchers.Main) { plugin.invoke(3, fields(listOf(id))) }
                awaitAbsent(manager, tag)
                withContext(Dispatchers.Main) {
                    plugin.invoke(2, fields(listOf(id, "Cancel all", "", channel)))
                }
                awaitNotification(manager, tag, "Cancel all")
                withContext(Dispatchers.Main) { plugin.invoke(6, ByteBuffer.allocate(0)) }
                awaitAbsent(manager, tag)
                assertEquals(true, manager.activeNotifications.any { it.tag == unrelatedTag })
                withContext(Dispatchers.Main) {
                    plugin.invoke(2, fields(listOf(id, "Persists after close", "", channel)))
                    plugin.close()
                    try {
                        plugin.invoke(0, ByteBuffer.allocate(0))
                        fail("Closed handler accepted a query")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.CANCELLED, error.code)
                    }
                }
                awaitNotification(manager, tag, "Persists after close")
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
                manager.cancel(tag, 0)
                manager.cancel(unrelatedTag, 42)
                manager.deleteNotificationChannel(channelId)
            }
        }

    @Test
    fun malformedPayloadsNeverPost() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val plugin = withContext(Dispatchers.Main) { NotificationsPlugin(context) }
            try {
                withContext(Dispatchers.Main) {
                    val invalid =
                        listOf(
                            fields(listOf("id", "", "body", "general")),
                            fields(listOf("id", "Title", "bad\u0000body", "general")),
                            ByteBuffer.wrap(byteArrayOf(-1, -1, -1, -1)),
                        )

                    for (payload in invalid) {
                        try {
                            plugin.invoke(2, payload)
                            fail("Invalid content must be rejected before posting")
                        } catch (error: PluginException) {
                            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                        }
                    }

                    try {
                        plugin.invoke(4, fields(listOf("invalid-channel", "Test", ""), listOf(4, 0)))
                        fail("Unknown channel importance was accepted")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                    }
                }
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
            }
        }

    /** A registration whose registrar feeds the broker answers the new
     * snapshot — wire state 4, Firebase's exact bytes — and no description
     * of the broker's state carries those bytes. Reading before any answer
     * reports the empty snapshot without asking the OS. */
    @Test
    fun remoteRegistrationAnswersTheBrokerThroughABoundedWait() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val token = "fcm-${System.nanoTime()}:A-z_09"
            RemoteRegistration.reset()
            val plugin =
                withContext(Dispatchers.Main) {
                    NotificationsPlugin(context, null, { RemoteRegistration.noteToken(token) }, 5_000)
                }
            try {
                withContext(Dispatchers.Main) {
                    val reply = snapshot(plugin.invoke(7, ByteBuffer.allocate(0)))
                    assertEquals(0L, reply.revision)
                    assertEquals(0, reply.state)
                    assertEquals("", reply.text)

                    val registered = snapshot(plugin.invoke(8, ByteBuffer.allocate(0)))
                    assertEquals(1L, registered.revision)
                    assertEquals(4, registered.state)
                    assertEquals(token, registered.text)
                }

                assertFalse(RemoteRegistration.current().toString().contains(token))
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
                RemoteRegistration.reset()
            }
        }

    /** The device method answers what the platform reports — the default
     * locale's tag, the default time zone's id, the package's version name
     * — as three length-prefixed strings, an unknown value as empty, and an
     * unshippable one as unknown rather than a refusal of the rest. */
    @Test
    fun theDeviceMethodAnswersWhatThePlatformReports() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val reported =
                withContext(Dispatchers.Main) {
                    NotificationsPlugin(context, null, {}, 5_000) {
                        DeviceDetails("en-GB", "Europe/London", "2.1.0")
                    }
                }
            val odd =
                withContext(Dispatchers.Main) {
                    NotificationsPlugin(context, null, {}, 5_000) {
                        DeviceDetails("en\u0007GB", null, "9".repeat(NotificationsPlugin.MAX_DEVICE_DETAIL_BYTES + 1))
                    }
                }
            val system = withContext(Dispatchers.Main) { NotificationsPlugin(context) }
            try {
                withContext(Dispatchers.Main) {
                    assertEquals(
                        listOf("en-GB", "Europe/London", "2.1.0"),
                        strings(reported.invoke(9, ByteBuffer.allocate(0))),
                    )
                    assertEquals(listOf("", "", ""), strings(odd.invoke(9, ByteBuffer.allocate(0))))

                    try {
                        reported.invoke(9, ByteBuffer.wrap(byteArrayOf(1)))
                        fail("a payload must be refused")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                    }

                    val details = strings(system.invoke(9, ByteBuffer.allocate(0)))
                    assertEquals(
                        java.util.Locale
                            .getDefault()
                            .toLanguageTag(),
                        details[0],
                    )
                    assertEquals(
                        java.util.TimeZone
                            .getDefault()
                            .id,
                        details[1],
                    )
                    assertEquals(
                        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "",
                        details[2],
                    )
                }
            } finally {
                withContext(Dispatchers.Main) {
                    reported.close()
                    odd.close()
                    system.close()
                }
            }
        }

    /** Silence at the bound is an explicit non-answer: revision 0, state
     * Unconfirmed, never the broker's previous token dressed up as fresh.
     * That previous answer stays readable through `REMOTE_TOKEN`. */
    @Test
    fun aSilentRegistrationAnswersUnconfirmedAndNeverAStaleToken() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val stale = "fcm-stale-${System.nanoTime()}"
            RemoteRegistration.reset()
            RemoteRegistration.noteToken(stale)
            val plugin =
                withContext(Dispatchers.Main) {
                    NotificationsPlugin(context, null, { /* the OS stays silent */ }, 150)
                }
            try {
                withContext(Dispatchers.Main) {
                    val reply = snapshot(plugin.invoke(8, ByteBuffer.allocate(0)))
                    assertEquals(0L, reply.revision)
                    assertEquals(3, reply.state)
                    assertEquals("", reply.text)

                    val read = snapshot(plugin.invoke(7, ByteBuffer.allocate(0)))
                    assertEquals(1L, read.revision)
                    assertEquals(4, read.state)
                    assertEquals(stale, read.text)
                }
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
                RemoteRegistration.reset()
            }
        }

    /** A token outside the contract's bounds is an honest failure, never a
     * shortened or padded token, and its bytes never reach the reason. */
    @Test
    fun anUnrepresentableTokenBecomesAFailureWithoutLeakingBytes() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val unrepresentable = "fcm-" + "x".repeat(4_097)
            RemoteRegistration.reset()
            val plugin =
                withContext(Dispatchers.Main) {
                    NotificationsPlugin(context, null, { RemoteRegistration.noteToken(unrepresentable) }, 5_000)
                }
            try {
                withContext(Dispatchers.Main) {
                    val reply = snapshot(plugin.invoke(8, ByteBuffer.allocate(0)))
                    assertEquals(1L, reply.revision)
                    assertEquals(2, reply.state)
                    assertEquals("token-shape", reply.text)
                }

                assertFalse(RemoteRegistration.current().toString().contains(unrepresentable))
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
                RemoteRegistration.reset()
            }
        }

    /** A plugin closed while its registration waits never answers a token:
     * the close is checked after the wait, so a late callback lands in the
     * broker without reaching the caller that left. */
    @Test
    fun closingWhileWaitingNeverAnswersAToken() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            RemoteRegistration.reset()
            var subject: NotificationsPlugin? = null
            val plugin =
                withContext(Dispatchers.Main) {
                    NotificationsPlugin(context, null, { subject?.close() }, 150)
                        .also { subject = it }
                }
            try {
                withContext(Dispatchers.Main) {
                    try {
                        plugin.invoke(8, ByteBuffer.allocate(0))
                        fail("A closed plugin answered a registration")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.CANCELLED, error.code)
                    }
                }
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
                RemoteRegistration.reset()
            }
        }

    /** Callers that arrive while one registration runs join its wait: the OS
     * is asked once, and every waiter sees the same answer. */
    @Test
    fun concurrentRegistrationsAskOnceAndSeeTheSameAnswer() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val token = "fcm-shared-${System.nanoTime()}"
            val asks = AtomicInteger()
            RemoteRegistration.reset()
            val plugin =
                withContext(Dispatchers.Main) {
                    NotificationsPlugin(
                        context,
                        null,
                        {
                            asks.incrementAndGet()
                            RemoteRegistration.noteToken(token)
                        },
                        5_000,
                    )
                }
            try {
                val waiters =
                    listOf(
                        async(Dispatchers.Main) { snapshot(plugin.invoke(8, ByteBuffer.allocate(0))) },
                        async(Dispatchers.Main) { snapshot(plugin.invoke(8, ByteBuffer.allocate(0))) },
                    )
                // The first caller asks and starts waiting; the broker's
                // answer reaches both through the shared wait.
                withTimeout(5000) {
                    while (asks.get() == 0) delay(10)
                }
                RemoteRegistration.noteToken(token)

                for (waiter in waiters) {
                    val reply = waiter.await()
                    assertEquals(1L, reply.revision)
                    assertEquals(4, reply.state)
                    assertEquals(token, reply.text)
                }
                assertEquals(1, asks.get())
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
                RemoteRegistration.reset()
            }
        }

    /** A push that names no channel posts through the one the manifest
     * names as Firebase's default, so that channel has to exist: the plugin
     * creates it in its own namespace, once, and leaves an existing one —
     * the app's own wording, the person's own settings — exactly as it is.
     * An app that isn't push-enabled names none, and gets none. */
    @Test
    fun theDefaultPushChannelExistsOnceAndIsNeverOverwritten() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val manager = context.getSystemService(NotificationManager::class.java)
            val id = NotificationsPlugin.PREFIX + "default-${System.nanoTime()}"
            val plugin = withContext(Dispatchers.Main) { NotificationsPlugin(context) }
            try {
                // The fixture app isn't push-enabled: its manifest names no
                // default channel, so constructing the plugin made none.
                assertNull(manager.getNotificationChannel(NotificationsPlugin.PREFIX + "default"))

                plugin.ensurePushChannel(manager, null)
                plugin.ensurePushChannel(manager, "someone-elses-channel")
                plugin.ensurePushChannel(manager, NotificationsPlugin.PREFIX)
                assertNull(manager.getNotificationChannel("someone-elses-channel"))
                assertNull(manager.getNotificationChannel(NotificationsPlugin.PREFIX))

                plugin.ensurePushChannel(manager, id)
                val created = manager.getNotificationChannel(id)
                assertNotNull(created)
                assertEquals(NotificationManager.IMPORTANCE_DEFAULT, created.importance)
                assertEquals(NotificationsPlugin.DEFAULT_PUSH_CHANNEL_NAME, created.name.toString())

                // The app's own configuration wins from then on.
                withContext(Dispatchers.Main) {
                    plugin.invoke(
                        4,
                        fields(listOf(id.removePrefix(NotificationsPlugin.PREFIX), "Updates", "From the team"), listOf(1, 0)),
                    )
                }
                plugin.ensurePushChannel(manager, id)
                assertEquals("Updates", manager.getNotificationChannel(id).name.toString())
            } finally {
                manager.deleteNotificationChannel(id)
                withContext(Dispatchers.Main) { plugin.close() }
            }
        }

    /** Firebase without initialization values throws `IllegalStateException`
     * from a method the plugin calls through reflection, which hands it over
     * wrapped: the wrapper is opened, so the answer is
     * `firebase-not-configured` and not the catch-all `os-refused`. */
    @Test
    fun aWrappedFirebaseFailureKeepsItsStableCode() {
        val notConfigured = RemoteRegistration.FailureCode.FIREBASE_NOT_CONFIGURED
        val refused = RemoteRegistration.FailureCode.OS_REFUSED

        assertEquals(
            notConfigured,
            SystemFirebaseRegistrar.registrationFailure(
                InvocationTargetException(IllegalStateException("Default FirebaseApp is not initialized")),
            ),
        )
        assertEquals(notConfigured, SystemFirebaseRegistrar.registrationFailure(IllegalStateException()))
        assertEquals(notConfigured, SystemFirebaseRegistrar.registrationFailure(ClassNotFoundException()))
        assertEquals(notConfigured, SystemFirebaseRegistrar.registrationFailure(NoClassDefFoundError()))
        assertEquals(
            refused,
            SystemFirebaseRegistrar.registrationFailure(InvocationTargetException(SecurityException())),
        )
        assertEquals(refused, SystemFirebaseRegistrar.registrationFailure(NoSuchMethodException()))
    }

    /** The real registrar in an app without Firebase — this fixture carries
     * no `firebase-messaging` — answers the stable code instead of throwing
     * or leaving the broker empty. */
    @Test
    fun theRealRegistrarWithoutFirebaseAnswersNotConfigured() {
        RemoteRegistration.reset()
        try {
            SystemFirebaseRegistrar.register()

            assertEquals(
                RemoteRegistration.State.Failed(RemoteRegistration.FailureCode.FIREBASE_NOT_CONFIGURED),
                RemoteRegistration.current().state,
            )
        } finally {
            RemoteRegistration.reset()
        }
    }

    /** One snapshot reply, decoded: revision `u64`, state `u32`, string. */
    private class SnapshotReply(
        val revision: Long,
        val state: Int,
        val text: String,
    )

    private fun snapshot(reply: ByteArray): SnapshotReply {
        val data = ByteBuffer.wrap(reply).order(ByteOrder.LITTLE_ENDIAN)
        val revision = (data.int.toLong() shl 32) or (data.int.toLong() and 0xFFFFFFFFL)
        val state = data.int
        val bytes = ByteArray(data.int)

        data.get(bytes)

        return SnapshotReply(revision, state, String(bytes, Charsets.UTF_8))
    }

    private suspend fun awaitNotification(
        manager: NotificationManager,
        tag: String,
        title: String,
    ) = withTimeout(5000) {
        while (manager.activeNotifications.none {
                it.tag == tag && it.notification.extras
                    .getCharSequence(Notification.EXTRA_TITLE)
                    ?.toString() == title
            }
        ) {
            delay(20)
        }
    }

    private suspend fun awaitAbsent(
        manager: NotificationManager,
        tag: String,
    ) = withTimeout(5000) {
        while (manager.activeNotifications.any { it.tag == tag }) delay(20)
        assertFalse(manager.activeNotifications.any { it.tag == tag })
    }

    /** A reply of length-prefixed strings, decoded, with nothing left over. */
    private fun strings(reply: ByteArray): List<String> {
        val data = ByteBuffer.wrap(reply).order(ByteOrder.LITTLE_ENDIAN)
        val strings = mutableListOf<String>()

        while (data.hasRemaining()) {
            val bytes = ByteArray(data.int)

            data.get(bytes)
            strings.add(String(bytes, Charsets.UTF_8))
        }

        return strings
    }

    private fun fields(
        strings: List<String>,
        numbers: List<Int> = emptyList(),
    ): ByteBuffer {
        val output = ByteArrayOutputStream()

        for (value in strings) {
            val bytes = value.toByteArray(Charsets.UTF_8)
            output.write(
                ByteBuffer
                    .allocate(4)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(bytes.size)
                    .array(),
            )
            output.write(bytes)
        }

        for (value in numbers) {
            output.write(
                ByteBuffer
                    .allocate(4)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(value)
                    .array(),
            )
        }

        return ByteBuffer.wrap(output.toByteArray())
    }
}
