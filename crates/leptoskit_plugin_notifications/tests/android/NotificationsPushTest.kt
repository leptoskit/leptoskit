package dev.leptoskit.plugins.notifications

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.PackageManager
import android.os.Build
import android.service.notification.StatusBarNotification
import androidx.test.platform.app.InstrumentationRegistry
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.RemoteMessage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Before
import org.junit.Test

/**
 * Runs only in the push-enabled fixture app (`native_plugin_suite --push`),
 * against the `LeptosKitMessagingService` that `[native.android] push`
 * generates and compiles against Firebase. Google Play services never
 * delivers anything here: the configuration is fake, so the test calls the
 * service's callbacks itself, with messages Firebase's own builder makes.
 */
class NotificationsPushTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context: Context = instrumentation.targetContext
    private val manager = context.getSystemService(NotificationManager::class.java)

    @Before
    fun resetBefore() {
        RemoteRegistration.reset()
        ReceivedNotifications.reset()
    }

    @After
    fun resetAfter() {
        RemoteRegistration.reset()
        ReceivedNotifications.reset()
    }

    /** The app's Firebase is configured from its `google-services.json`:
     * generation applied Google services' Gradle plugin, which turned the
     * fake file the fixture wrote into the values Firebase starts from.
     * Without it `FirebaseApp` is never initialized, and the plugin's
     * registrar answers `firebase-not-configured`. */
    @Test
    fun firebaseStartsFromTheAppsGoogleServicesFile() {
        val options = FirebaseApp.getInstance().options

        assertEquals("leptoskit-ci-fake", options.projectId)
        assertEquals("1:000000000000:android:0000000000000000000000", options.applicationId)
    }

    /** A refreshed registration token reaches the plugin's broker exactly
     * as Firebase issued it. */
    @Test
    fun aRefreshedTokenLandsInTheRegistrationBroker() {
        val token = "ci-fake-fcm-token:${System.nanoTime()}"

        service().onNewToken(token)

        assertEquals(RemoteRegistration.State.Token(token), RemoteRegistration.current().state)
    }

    /** A data message is kept for the app's code as its data map — the
     * envelope's keys and the send's own, none of Firebase's — and nothing
     * is posted. */
    @Test
    fun aDataMessageIsKeptForTheAppAndPostsNothing() {
        val id = notificationId()
        val message =
            RemoteMessage
                .Builder("leptoskit-ci-fake@fcm.googleapis.com")
                .setMessageId("0:ci-data-$id")
                .addData("lkn", id)
                .addData("lklink", "/orders/7")
                .addData("sync", "orders")
                .build()

        service().onMessageReceived(message)

        val items = ReceivedNotifications.take()
        assertEquals(listOf(ReceivedNotifications.MESSAGE), items.map { it.kind })
        val payload = JSONObject(items[0].payload)
        assertEquals(setOf("lkn", "lklink", "sync"), payload.keys().asSequence().toSet())
        assertEquals(id, payload.getString("lkn"))
        assertEquals("/orders/7", payload.getString("lklink"))
        assertEquals("orders", payload.getString("sync"))
        assertFalse(manager.activeNotifications.any { it.tag == ReceivedNotifications.PUSH_TAG_PREFIX + id })
    }

    /** A message with a notification part reaches the service only while
     * the app is open. It names no channel, so the plugin posts it through
     * the one the generated manifest names as Firebase's default — which
     * the plugin creates when it starts — and records that it arrived. */
    @Test
    fun aPushWhileTheAppIsOpenIsPostedThroughTheManifestsDefaultChannel() =
        runBlocking {
            if (Build.VERSION.SDK_INT >= 33 &&
                context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
            ) {
                // This permission belongs to the disposable test app and lasts until fixture uninstall.
                instrumentation.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
            }
            val channel = NotificationsPlugin.PREFIX + "default"
            val id = notificationId()
            val tag = ReceivedNotifications.PUSH_TAG_PREFIX + id
            val plugin = withContext(Dispatchers.Main) { NotificationsPlugin(context) }
            try {
                assertEquals(channel, NotificationsPlugin.manifestPushChannel(context))
                assertNotNull(manager.getNotificationChannel(channel))

                // The notification part travels as FCM delivers it: `gcm.n.`
                // keys beside the data, which `RemoteMessage` parses into its
                // notification and keeps out of the data map.
                val message =
                    RemoteMessage
                        .Builder("leptoskit-ci-fake@fcm.googleapis.com")
                        .setMessageId("0:ci-notification-$id")
                        .addData("lkn", id)
                        .addData("lklink", "/invoice/42")
                        .addData("invoice", "42")
                        .addData("gcm.n.e", "1")
                        .addData("gcm.n.title", "Paid")
                        .addData("gcm.n.body", "Invoice 42")
                        .build()

                service().onMessageReceived(message)

                val posted = awaitNotification(tag, "Paid")
                assertEquals(channel, posted.notification.channelId)
                assertEquals(
                    "Invoice 42",
                    posted.notification.extras
                        .getCharSequence(Notification.EXTRA_TEXT)
                        ?.toString(),
                )

                val items = ReceivedNotifications.take()
                assertEquals(listOf(ReceivedNotifications.PRESENTED), items.map { it.kind })
                val payload = JSONObject(items[0].payload)
                assertEquals(setOf("lkn", "lklink", "invoice"), payload.keys().asSequence().toSet())
                assertEquals(id, payload.getString("lkn"))
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
                manager.cancel(tag, 0)
                manager.deleteNotificationChannel(channel)
            }
        }

    /** The generated service, as the system would start it: attached to the
     * app's context. `attachBaseContext` is protected and the system calls
     * it through a hidden `Service.attach`, so the test reaches it through
     * reflection on `ContextWrapper`, where it is declared. */
    private fun service(): LeptosKitMessagingService {
        val service = LeptosKitMessagingService()

        ContextWrapper::class.java
            .getDeclaredMethod("attachBaseContext", Context::class.java)
            .apply { isAccessible = true }
            .invoke(service, context)

        return service
    }

    /** A notification id in the service's UUID shape, unique per run. */
    private fun notificationId(): String = "0190a1b2-0000-7000-8000-${System.nanoTime().toString().takeLast(12).padStart(12, '0')}"

    private suspend fun awaitNotification(
        tag: String,
        title: String,
    ): StatusBarNotification =
        withTimeout(5000) {
            var found = posted(tag, title)

            while (found == null) {
                delay(20)
                found = posted(tag, title)
            }

            checkNotNull(found)
        }

    private fun posted(
        tag: String,
        title: String,
    ): StatusBarNotification? =
        manager.activeNotifications.firstOrNull {
            it.tag == tag &&
                it.notification.extras
                    .getCharSequence(Notification.EXTRA_TITLE)
                    ?.toString() == title
        }
}
