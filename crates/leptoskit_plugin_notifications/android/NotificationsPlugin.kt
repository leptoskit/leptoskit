package dev.leptoskit.plugins.notifications

import android.Manifest
import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.os.Build
import android.os.Looper
import dev.leptoskit.bridge.NativePermissionHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import java.lang.reflect.InvocationTargetException
import java.lang.reflect.Proxy
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.util.Locale
import java.util.TimeZone
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The latest remote-registration answer the app forwarded. An object on
 * purpose: answers can arrive before the plugin is constructed and its guest
 * exists, a closed plugin must leave the OS's answer readable, and nothing
 * here retains a plugin instance. The token is held only in memory: the
 * process asks the OS again on the next launch rather than treating a stored
 * copy as authoritative.
 */
public object RemoteRegistration {
    /** Bounds shared with the Rust contract: the token is variable length and
     * never assumed to be a hex string of any size. */
    internal const val MAX_TOKEN_CHARACTERS = 4_096
    internal const val MAX_FAILURE_BYTES = 256

    /** The registration answer's shape. Descriptions never print a token's
     * bytes. */
    public sealed interface State {
        /** No answer has arrived since the host process started. */
        public data object None : State

        /** The OS issued a registration token, exactly as issued. */
        public data class Token(
            public val value: String,
        ) : State {
            override fun toString(): String = "token(<${value.length} chars>)"
        }

        /** The OS refused registration; one of the contract's stable codes. */
        public data class Failed(
            public val reason: String,
        ) : State

        /** A registration wait ended without a new answer. The broker's last
         * answer, if any, is unchanged and still readable. */
        public data object Unconfirmed : State
    }

    /** The registration answer: a monotonic revision and the latest state.
     * Revision `0` means no answer has arrived since the process started. */
    public data class Snapshot(
        public val revision: Long,
        public val state: State,
    ) {
        override fun toString(): String = "Snapshot(revision: $revision, state: $state)"
    }

    @Volatile
    private var snapshot: Snapshot = Snapshot(0, State.None)

    /** Reads the latest answer without side effects. */
    @Synchronized
    public fun current(): Snapshot = snapshot

    /**
     * The messaging service's `onNewToken`, and the registration task's own
     * answer. Firebase's bytes stay exactly as issued: printable ASCII of no
     * assumed length. A token outside the contract's bounds is recorded as a
     * failure instead of a token, and its bytes never reach a log.
     */
    @Synchronized
    public fun noteToken(token: String) {
        val shaped =
            token.isNotEmpty() &&
                token.length <= MAX_TOKEN_CHARACTERS &&
                token.all { it.code in 0x21..0x7E }

        if (!shaped) {
            note(FailureCode.TOKEN_SHAPE)
            return
        }

        snapshot = Snapshot(nextRevision(), State.Token(token))
    }

    /** The registration boundary's refusal, as a stable code. */
    @Synchronized
    public fun noteFailure(code: String) {
        note(code)
    }

    /** The contract's stable failure codes, mirrored from the Rust side. */
    internal object FailureCode {
        const val OS_REFUSED = "os-refused"
        const val FIREBASE_NOT_CONFIGURED = "firebase-not-configured"
        const val TOKEN_SHAPE = "token-shape"
    }

    /** Maps the SDK's own failure to its stable code. The one distinction
     * worth keeping is Firebase without initialization values — the app
     * author's `google-services.json` is missing — matched on the exception
     * family, never on error text. */
    internal fun failureCode(error: Throwable?): String =
        when (error) {
            null -> FailureCode.OS_REFUSED
            is IllegalStateException -> FailureCode.FIREBASE_NOT_CONFIGURED
            else -> FailureCode.OS_REFUSED
        }

    @Synchronized
    internal fun note(code: String) {
        snapshot = Snapshot(nextRevision(), State.Failed(code))
    }

    /** The next revision, saturating instead of trapping should the counter
     * ever reach the top. */
    private fun nextRevision(): Long {
        val next = snapshot.revision + 1
        return if (next == 0L) Long.MAX_VALUE else next
    }

    /** Resets the broker. Process-scoped state, so only tests call this. */
    @Synchronized
    public fun reset(to: Snapshot = Snapshot(0, State.None)) {
        snapshot = to
    }
}

/**
 * The registration boundary production asks through Google Play services.
 * Tests substitute a fake.
 */
internal fun interface FirebaseRegistrar {
    /**
     * Asks Firebase for the registration token. Every failure lands in the
     * broker as a stable code, never the SDK's own text, and a late token
     * lands there too, exactly as a refresh would.
     */
    fun register()
}

/** Asks `FirebaseMessaging` for its token without a compile-time Firebase
 * dependency: the app carries `firebase-messaging` only when
 * `[native.android] push` opts it in, so the classes resolve reflectively and
 * their absence is the contract's own `firebase-not-configured` answer. A
 * push-enabled release build keeps these names from R8 with the rules
 * generation writes beside the bridge's own. Tokens cross into the broker
 * and are never logged. */
internal object SystemFirebaseRegistrar : FirebaseRegistrar {
    private const val MESSAGING = "com.google.firebase.messaging.FirebaseMessaging"
    private const val TASK = "com.google.android.gms.tasks.Task"
    private const val SUCCESS = "com.google.android.gms.tasks.OnSuccessListener"
    private const val FAILURE = "com.google.android.gms.tasks.OnFailureListener"

    override fun register() {
        try {
            val loader = RemoteRegistration::class.java.classLoader
            val messaging = Class.forName(MESSAGING, false, loader)
            val instance = messaging.getMethod("getInstance").invoke(null)
            val task = messaging.getMethod("getToken").invoke(instance)
            val tasks = Class.forName(TASK, false, loader)
            val success = Class.forName(SUCCESS, false, loader)
            val failure = Class.forName(FAILURE, false, loader)
            val listener =
                Proxy.newProxyInstance(loader, arrayOf(success, failure)) { proxy, method, arguments ->
                    when (method.name) {
                        "onSuccess" -> {
                            RemoteRegistration.noteToken(arguments?.get(0) as? String ?: "")
                        }

                        "onFailure" -> {
                            RemoteRegistration.noteFailure(
                                RemoteRegistration.failureCode(arguments?.get(0) as? Throwable),
                            )
                        }

                        "toString" -> {
                            "LeptosKitTokenListener"
                        }

                        "hashCode" -> {
                            System.identityHashCode(proxy)
                        }

                        "equals" -> {
                            proxy === arguments?.get(0)
                        }

                        else -> {
                            null
                        }
                    }
                }
            // Looked up on the public `Task` type, not on the returned
            // task's own class: the SDK's implementation class is its own
            // business, and reflection refuses a method found through a
            // class that isn't public.
            tasks.getMethod("addOnSuccessListener", success).invoke(task, listener)
            tasks.getMethod("addOnFailureListener", failure).invoke(task, listener)
        } catch (absent: NoClassDefFoundError) {
            RemoteRegistration.noteFailure(registrationFailure(absent))
        } catch (refused: Exception) {
            RemoteRegistration.noteFailure(registrationFailure(refused))
        }
    }

    /**
     * Maps what the reflective registration threw to its stable code. A
     * method called through reflection reports what it threw wrapped in an
     * `InvocationTargetException`, so the wrapper is opened first: Firebase
     * without initialization values throws `IllegalStateException` from
     * `getInstance`, and that is `firebase-not-configured` however it
     * arrives. Missing Firebase classes are the same answer; everything
     * else is the OS's refusal. Matched on the exception family, never on
     * error text.
     */
    internal fun registrationFailure(error: Throwable): String {
        val cause = if (error is InvocationTargetException) error.targetException ?: error else error

        return when (cause) {
            is ClassNotFoundException,
            is NoClassDefFoundError,
            is IllegalStateException,
            -> RemoteRegistration.FailureCode.FIREBASE_NOT_CONFIGURED

            else -> RemoteRegistration.FailureCode.OS_REFUSED
        }
    }
}

/** What the device says about itself for push targeting, as `DEVICE`
 * answers it. A `null` value is unknown. */
internal data class DeviceDetails(
    val locale: String?,
    val timeZone: String?,
    val appVersion: String?,
)

/** Where the device's details come from. Production reads the platform;
 * tests substitute a fake. */
internal fun interface DeviceFacts {
    fun current(): DeviceDetails
}

/** The platform's own answers: the default locale's language tag, the
 * default time zone's id, and the package's version name — read fresh on
 * every call, so a language change or an app update shows on the next
 * read. */
internal class SystemDeviceFacts(
    private val context: Context,
) : DeviceFacts {
    override fun current(): DeviceDetails =
        DeviceDetails(
            locale = Locale.getDefault().toLanguageTag(),
            timeZone = TimeZone.getDefault().id,
            appVersion = versionName(),
        )

    private fun versionName(): String? =
        try {
            @Suppress("DEPRECATION")
            context.packageManager.getPackageInfo(context.packageName, 0).versionName
        } catch (_: PackageManager.NameNotFoundException) {
            null
        }
}

/** Immediate notifications only. Posting never implicitly requests permission. */
class NotificationsPlugin internal constructor(
    private val context: Context,
    smallIcon: Int?,
    private val registrar: FirebaseRegistrar,
    private val remoteWaitLimitMillis: Long,
    private val device: DeviceFacts = SystemDeviceFacts(context),
) : NativePlugin {
    /** The constructor the generated app and ordinary registrations use: the
     * registration boundary is the real one, and the OS's own answer bound
     * applies. */
    internal constructor(
        context: Context,
        smallIcon: Int? = null,
    ) : this(context, smallIcon, SystemFirebaseRegistrar, DEFAULT_REMOTE_WAIT_MILLIS)

    private val smallIcon: Int = smallIcon ?: resolveSmallIcon(context)

    override val name = "notifications"
    override val methods =
        intArrayOf(STATE, REQUEST, SHOW, CANCEL, CHANNEL, DELETE, ACTIVE, REMOTE_TOKEN, REMOTE_REGISTER, DEVICE)

    companion object {
        /** The notification tag and channel prefix the plugin owns. */
        internal const val PREFIX = "leptoskit.notifications."

        /** The manifest entry that names the channel Firebase posts a push
         * through when the send names none. Generation writes it, naming
         * `leptoskit.notifications.default`, while `[native.android] push`
         * is on. */
        internal const val DEFAULT_PUSH_CHANNEL_KEY = "com.google.firebase.messaging.default_notification_channel_id"

        /** What the default push channel is called in the system's settings
         * until the app configures the `default` channel with its own
         * wording. */
        internal const val DEFAULT_PUSH_CHANNEL_NAME = "Notifications"

        /**
         * Uses `@drawable/ic_notification` when `[app.icons]` generated one,
         * otherwise the system information icon.
         */
        private fun resolveSmallIcon(context: Context): Int {
            val id = context.resources.getIdentifier("ic_notification", "drawable", context.packageName)
            return if (id != 0) id else android.R.drawable.ic_dialog_info
        }

        /** The permission value that means notifications may be posted. */
        internal const val GRANTED = 3

        /** Reports the current permission, without asking for it. */
        const val STATE = 0

        /** Reports the permission, asking for it when it can. */
        const val REQUEST = 1

        /** Posts a notification to a configured channel. */
        const val SHOW = 2

        /** Cancels one posted notification. */
        const val CANCEL = 3

        /** Creates or replaces a channel. */
        const val CHANNEL = 4

        /** Deletes one channel. */
        const val DELETE = 5

        /** Lists the plugin's posted notifications. */
        const val ACTIVE = 6

        /** Reads the broker's latest remote-registration answer, without side
         * effects. */
        const val REMOTE_TOKEN = 7

        /** Asks the OS to register and waits a bounded time for a new
         * answer. */
        const val REMOTE_REGISTER = 8

        /** Reads what the device says about itself for push targeting. */
        const val DEVICE = 9

        /** The longest device detail a `DEVICE` reply carries, in bytes,
         * matching the Rust contract's `MAX_DEVICE_DETAIL_BYTES`. */
        internal const val MAX_DEVICE_DETAIL_BYTES = 256

        /** How long `REMOTE_REGISTER` waits for the OS answer before
         * replying with the unchanged snapshot, in milliseconds. Firebase
         * answers a cached token quickly; the bound keeps a silent OS from
         * hanging the call. */
        internal const val DEFAULT_REMOTE_WAIT_MILLIS = 10_000L

        /** How often the bounded wait re-reads the broker, in milliseconds. */
        internal const val REMOTE_WAIT_SLICE_MILLIS = 50L

        /** The largest show request: an id, title, body, and channel. */
        const val SHOW_BYTES = 4496

        /** The largest cancel or delete request: one identifier. */
        const val ID_BYTES = 68

        /** The largest channel request. */
        const val CHANNEL_BYTES = 1236

        /** Longest title and body the wire format carries. */
        const val MAX_TITLE = 256
        const val MAX_BODY = 4096
    }

    private val manager = context.getSystemService(NotificationManager::class.java)
    private var closed = false
    private val registering = AtomicBoolean(false)

    init {
        manager?.let { ensurePushChannel(it, manifestPushChannel()) }
    }

    /** The channel the app's manifest names as Firebase's default, when it
     * names one: only a push-enabled app does. */
    private fun manifestPushChannel(): String? =
        try {
            @Suppress("DEPRECATION")
            context.packageManager
                .getApplicationInfo(context.packageName, PackageManager.GET_META_DATA)
                .metaData
                ?.getString(DEFAULT_PUSH_CHANNEL_KEY)
        } catch (_: PackageManager.NameNotFoundException) {
            null
        }

    /**
     * Makes the default push channel exist. Firebase posts a push that
     * names no channel — or one that doesn't exist — through the channel
     * the manifest names, and falls back to a "Miscellaneous" channel of
     * its own when that one is missing too. Only a channel in the plugin's
     * own namespace is created, and one that already exists is left exactly
     * as the app or the person set it: `configure_channel` with the id
     * `default` renames and reshapes it.
     */
    internal fun ensurePushChannel(
        manager: NotificationManager,
        id: String?,
    ) {
        if (id == null || !id.startsWith(PREFIX) || id.length == PREFIX.length) return
        if (manager.getNotificationChannel(id) != null) return

        manager.createNotificationChannel(
            NotificationChannel(id, DEFAULT_PUSH_CHANNEL_NAME, NotificationManager.IMPORTANCE_DEFAULT),
        )
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper())

        if (closed) throw PluginException(PluginException.CANCELLED)
        currentCoroutineContext().ensureActive()
        val manager = manager ?: throw PluginException(PluginException.UNAVAILABLE)

        return dispatch(manager, method, payload)
    }

    /** Runs one method, and answers with the bytes it returns. */
    private suspend fun dispatch(
        manager: NotificationManager,
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        when (method) {
            STATE, REQUEST -> return permissionReply(manager, method, payload)
            SHOW -> show(manager, payload)
            CANCEL, DELETE -> cancelOrDelete(manager, method, payload)
            CHANNEL -> configure(Reader(payload, CHANNEL_BYTES), manager)
            ACTIVE -> active(manager, payload)
            REMOTE_TOKEN -> return remoteToken(payload)
            REMOTE_REGISTER -> return registerRemote(payload)
            DEVICE -> return deviceDetails(payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }

        return byteArrayOf()
    }

    /** Answers what the device says about itself, without side effects. */
    private fun deviceDetails(payload: ByteBuffer): ByteArray {
        empty(payload)

        return encodeDevice(device.current())
    }

    /** Encodes a `DEVICE` reply: three length-prefixed UTF-8 strings —
     * locale, time zone, app version — each empty when unknown. A value
     * past the contract's bound, or carrying a control character, is sent
     * as unknown rather than refused, so one odd detail never costs the
     * others. */
    internal fun encodeDevice(details: DeviceDetails): ByteArray {
        val data = ByteBuffer.allocate(12 + 3 * MAX_DEVICE_DETAIL_BYTES).order(ByteOrder.LITTLE_ENDIAN)

        for (value in listOf(details.locale, details.timeZone, details.appVersion)) {
            val bytes = shippable(value)?.toByteArray(Charsets.UTF_8) ?: byteArrayOf()

            data.putInt(bytes.size)
            data.put(bytes)
        }

        return data.array().copyOf(data.position())
    }

    /** A detail the contract can carry, or `null`. */
    private fun shippable(value: String?): String? =
        value?.takeIf {
            it.toByteArray(Charsets.UTF_8).size <= MAX_DEVICE_DETAIL_BYTES && it.none(Char::isISOControl)
        }

    /** Encodes the broker's latest answer: reading never asks and never
     * registers, so it is safe at launch. */
    private fun remoteToken(payload: ByteBuffer): ByteArray {
        empty(payload)

        return encodeSnapshot(RemoteRegistration.current())
    }

    /**
     * Asks the OS to register for remote notifications, then waits a bounded
     * time for the answer the registration task or the messaging service
     * forwards into the broker. One registration is in flight at a time:
     * callers that arrive while one runs join its wait rather than asking
     * the OS again. A silent OS answers `unconfirmed`, never the previous
     * token as though it were fresh — a late callback still lands in the
     * broker for the next read, and a plugin closed while waiting never
     * succeeds.
     */
    private suspend fun registerRemote(payload: ByteBuffer): ByteArray {
        empty(payload)
        if (closed) throw PluginException(PluginException.CANCELLED)
        currentCoroutineContext().ensureActive()

        val since = RemoteRegistration.current().revision

        // Bound the OS requests at one: the first caller asks, the rest wait
        // for whatever answer its registration produces.
        val asking = registering.compareAndSet(false, true)

        try {
            if (asking) registrar.register()

            val snapshot = waitRemoteChange(since)

            if (closed) throw PluginException(PluginException.CANCELLED)
            currentCoroutineContext().ensureActive()

            // Silence at the bound is an explicit non-answer, not a stale
            // token.
            return if (snapshot.revision == since) {
                encodeSnapshot(RemoteRegistration.Snapshot(0, RemoteRegistration.State.Unconfirmed))
            } else {
                encodeSnapshot(snapshot)
            }
        } finally {
            if (asking) registering.set(false)
        }
    }

    /**
     * Polls the broker until its revision moves past `since` or the wait
     * limit passes. Every waiter sees the same answer, the wait always
     * terminates, and cancellation ends it without touching the broker.
     */
    internal suspend fun waitRemoteChange(since: Long): RemoteRegistration.Snapshot {
        var waited = 0L

        while (true) {
            val snapshot = RemoteRegistration.current()

            if (snapshot.revision != since) return snapshot
            if (waited >= remoteWaitLimitMillis) return snapshot

            delay(REMOTE_WAIT_SLICE_MILLIS)
            waited += REMOTE_WAIT_SLICE_MILLIS
        }
    }

    /** Encodes a snapshot reply: revision `u64`, state `u32`, then a
     * length-prefixed string (empty for no answer). */
    internal fun encodeSnapshot(snapshot: RemoteRegistration.Snapshot): ByteArray {
        // Little-endian, as the bridge codec reads every integer: a
        // `ByteBuffer` is big-endian until told otherwise.
        val data = ByteBuffer.allocate(16 + RemoteRegistration.MAX_TOKEN_CHARACTERS).order(ByteOrder.LITTLE_ENDIAN)

        data.putInt((snapshot.revision ushr 32).toInt())
        data.putInt(snapshot.revision.toInt())

        when (val state = snapshot.state) {
            is RemoteRegistration.State.None -> {
                data.putInt(0)
                putString(data, "", RemoteRegistration.MAX_FAILURE_BYTES)
            }

            is RemoteRegistration.State.Token -> {
                // Android's answer is an FCM registration token: wire state
                // 4, the printable-ASCII kind. State 1 is the iOS hex kind.
                data.putInt(4)
                putString(data, state.value, RemoteRegistration.MAX_TOKEN_CHARACTERS)
            }

            is RemoteRegistration.State.Failed -> {
                data.putInt(2)
                putString(data, state.reason, RemoteRegistration.MAX_FAILURE_BYTES)
            }

            is RemoteRegistration.State.Unconfirmed -> {
                data.putInt(3)
                putString(data, "", RemoteRegistration.MAX_FAILURE_BYTES)
            }
        }

        return data.array().copyOf(data.position())
    }

    /** One length-prefixed UTF-8 string, bounded by the field's own
     * contract. */
    private fun putString(
        data: ByteBuffer,
        value: String,
        maximum: Int,
    ) {
        val bytes = value.toByteArray(Charsets.UTF_8)

        if (bytes.size > maximum) {
            throw PluginException(PluginException.TOO_LARGE)
        }
        data.putInt(bytes.size)
        data.put(bytes)
    }

    /** Reports the permission, asking for it when the caller asked to. */
    private suspend fun permissionReply(
        manager: NotificationManager,
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        empty(payload)

        if (method == REQUEST) requestPermission()

        return ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(permission())
            .array()
    }

    /** Posts one notification to a channel the app configured. */
    private fun show(
        manager: NotificationManager,
        payload: ByteBuffer,
    ) {
        val input = Reader(payload, SHOW_BYTES)
        val id = input.identifier()
        val title = input.text(MAX_TITLE, false, true)
        val body = input.text(MAX_BODY, true, false)
        val channel = input.identifier()
        input.finish()

        if (permission() != GRANTED) throw PluginException(PluginException.PERMISSION_DENIED)
        val configured =
            manager.getNotificationChannel(PREFIX + channel)
                ?: throw PluginException(PluginException.INVALID_ARGUMENT)

        if (configured.importance == NotificationManager.IMPORTANCE_NONE) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }

        val notification =
            Notification
                .Builder(context, configured.id)
                .setSmallIcon(smallIcon)
                .setContentTitle(title)
                .setContentText(body)
                .setStyle(Notification.BigTextStyle().bigText(body))
                .setAutoCancel(true)
                .build()

        try {
            manager.notify(PREFIX + id, 0, notification)
        } catch (_: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }
    }

    /** Cancels one posted notification, or deletes one channel. */
    private fun cancelOrDelete(
        manager: NotificationManager,
        method: Int,
        payload: ByteBuffer,
    ) {
        val input = Reader(payload, ID_BYTES)
        val id = input.identifier()
        input.finish()

        if (method == CANCEL) manager.cancel(PREFIX + id, 0) else manager.deleteNotificationChannel(PREFIX + id)
    }

    /** Cancels every notification this plugin posted. */
    private fun active(
        manager: NotificationManager,
        payload: ByteBuffer,
    ) {
        empty(payload)

        for (notification in manager.activeNotifications) {
            if (notification.tag?.startsWith(PREFIX) == true) manager.cancel(notification.tag, notification.id)
        }
    }

    override fun close() {
        closed = true
    }

    private fun permission(): Int {
        val manager = manager ?: return 0
        if (Build.VERSION.SDK_INT >= 33 &&
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            val activity = context as? Activity

            return if (activity?.shouldShowRequestPermissionRationale(Manifest.permission.POST_NOTIFICATIONS) == true) 2 else 0
        }

        return if (manager.areNotificationsEnabled()) 3 else 2
    }

    private suspend fun requestPermission() {
        if (Build.VERSION.SDK_INT < 33 || permission() == 3) return
        @Suppress("DEPRECATION")
        val declarations =
            context.packageManager
                .getPackageInfo(
                    context.packageName,
                    PackageManager.GET_PERMISSIONS,
                ).requestedPermissions
                .orEmpty()

        if (Manifest.permission.POST_NOTIFICATIONS !in declarations) throw PluginException(PluginException.INVALID_ARGUMENT)
        val host = context as? NativePermissionHost ?: throw PluginException(PluginException.UNAVAILABLE)
        val results = host.requestNativePermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS))

        if (closed) throw PluginException(PluginException.CANCELLED)

        currentCoroutineContext().ensureActive()

        if (results.size != 1) throw PluginException(PluginException.NATIVE)
    }

    private fun configure(
        input: Reader,
        manager: NotificationManager,
    ) {
        val id = input.identifier()
        val title = input.text(128, false, true)
        val description = input.text(1024, true, false)
        val importance =
            when (input.number()) {
                1 -> NotificationManager.IMPORTANCE_LOW
                2 -> NotificationManager.IMPORTANCE_DEFAULT
                3 -> NotificationManager.IMPORTANCE_HIGH
                else -> throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        val sound =
            when (input.number()) {
                0 -> false
                1 -> true
                else -> throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        input.finish()
        val channel = NotificationChannel(PREFIX + id, title, importance)
        channel.description = description
        channel.setSound(
            if (sound) RingtoneManager.getDefaultUri(RingtoneManager.TYPE_NOTIFICATION) else null,
            AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_NOTIFICATION).build(),
        )
        manager.createNotificationChannel(channel)
    }

    private fun empty(payload: ByteBuffer) {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}

private class Reader(
    payload: ByteBuffer,
    maximum: Int,
) {
    private val data = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

    init {
        if (data.remaining() > maximum) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    fun number(): Int {
        if (data.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)

        return data.int
    }

    fun identifier(): String {
        val value = text(64, false, true)

        if (!value.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it in "._-" }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return value
    }

    fun text(
        maximum: Int,
        multiline: Boolean,
        required: Boolean,
    ): String {
        val length = number()

        if (length !in 0..maximum || length > data.remaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        val field = data.slice().apply { limit(length) }
        data.position(data.position() + length)
        val value =
            try {
                Charsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(
                        CodingErrorAction.REPORT,
                    ).onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(field)
                    .toString()
            } catch (_: CharacterCodingException) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }

        if ((required && value.isBlank()) || value.any { it.isISOControl() && !(multiline && (it == '\n' || it == '\t')) }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return value
    }

    fun finish() {
        if (data.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}
