package dev.leptoskit.plugins.androidalarmmanager

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Looper
import android.provider.Settings
import android.util.Log
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import dev.leptoskit.mobile.GeneratedPlugins
import dev.leptoskit.mobile.NativeHeadlessGuest
import dev.leptoskit.mobile.NativeHeadlessOutcome
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import java.io.File
import java.io.IOException
import java.nio.ByteBuffer
import kotlin.coroutines.resume

private const val TAG = "LeptosKitAlarms"
internal const val ACTION_FIRE = "dev.leptoskit.plugins.androidalarmmanager.FIRE"
internal const val EXTRA_ID = "dev.leptoskit.plugins.androidalarmmanager.ID"

/**
 * Android `AlarmManager` alarms for the guest. Every call runs on the main
 * thread, as do the receivers, so the store needs no locking.
 *
 * Constructed for a headless launch, [context] is an [AlarmLaunchContext]:
 * this instance then reports the alarm that started the guest and ends the
 * run when the guest finishes it.
 */
class AndroidAlarmManagerPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "android_alarm_manager"
    override val methods =
        intArrayOf(STATUS, SCHEDULE, CANCEL, LIST, REQUEST_EXACT_PERMISSION, NEXT, DRAIN, LAUNCH, FINISH)
    private val application: Context = context.applicationContext ?: context
    private val launch = context as? AlarmLaunchContext
    private var launchSequence = launch?.record?.sequence ?: 0L
    private var waiting: Waiter? = null
    private var closed = false

    private class Waiter(
        val cursor: Long,
        val continuation: CancellableContinuation<ByteArray>,
    )

    init {
        AlarmRegistry.attach(this)
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)

        return when (method) {
            STATUS -> {
                status(payload)
            }

            SCHEDULE -> {
                schedule(decodeRequest(payload))
            }

            CANCEL -> {
                encodeFlag(AlarmRegistry.cancel(application, decodeCancel(payload)))
            }

            LIST -> {
                list(payload)
            }

            REQUEST_EXACT_PERMISSION -> {
                requestExact(payload)
            }

            NEXT -> {
                next(decodeNext(payload))
            }

            DRAIN -> {
                drain(payload)
            }

            LAUNCH -> {
                requireEmpty(payload)
                encodeLaunch(launch?.record?.takeIf { launchSequence != 0L })
            }

            FINISH -> {
                finish(decodeFinish(payload))
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }
    }

    override fun close() {
        closed = true
        AlarmRegistry.detach(this)
        val waiter = waiting
        waiting = null
        waiter?.continuation?.cancel()
    }

    private fun status(payload: ByteBuffer): ByteArray {
        requireEmpty(payload)
        val state = AlarmRegistry.state(application)

        return encodeStatus(
            Build.VERSION.SDK_INT,
            AlarmScheduler.canScheduleExact(application),
            NativeHeadlessGuest.available(application),
            state.fired.size,
            state.dropped,
        )
    }

    private fun schedule(request: AlarmRequest): ByteArray {
        val alarm = AlarmRegistry.schedule(application, request, System.currentTimeMillis())

        return encodeScheduled(alarm.timing, alarm.next)
    }

    private fun list(payload: ByteBuffer): ByteArray {
        requireEmpty(payload)

        return encodeList(AlarmRegistry.state(application).alarms)
    }

    private suspend fun requestExact(payload: ByteBuffer): ByteArray {
        requireEmpty(payload)

        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S || AlarmScheduler.canScheduleExact(application)) {
            return encodeFlag(true)
        }
        val host = context as? NativeActivityResultHost ?: throw PluginException(PluginException.UNAVAILABLE)
        val intent =
            Intent(Settings.ACTION_REQUEST_SCHEDULE_EXACT_ALARM, Uri.parse("package:${application.packageName}"))
        host.launchPluginActivity(intent)

        return encodeFlag(AlarmScheduler.canScheduleExact(application))
    }

    /** Acknowledges up to [cursor], then answers with the next fired alarm, waiting for one. */
    private suspend fun next(cursor: Long): ByteArray {
        if (waiting != null) throw PluginException(PluginException.BUSY)
        AlarmRegistry.acknowledge(application, cursor, launchSequence)
        AlarmRegistry.state(application).after(cursor, 1, launchSequence).firstOrNull()?.let {
            return encodeFired(it)
        }

        return suspendCancellableCoroutine { continuation ->
            val waiter = Waiter(cursor, continuation)
            waiting = waiter
            continuation.invokeOnCancellation { AlarmRegistry.onMain { if (waiting === waiter) waiting = null } }
        }
    }

    private fun drain(payload: ByteBuffer): ByteArray {
        val (cursor, max) = decodeDrain(payload)
        AlarmRegistry.acknowledge(application, cursor, launchSequence)

        return encodePage(AlarmRegistry.state(application).after(cursor, max, launchSequence))
    }

    private fun finish(sequence: Long): ByteArray {
        val owner = launch ?: throw PluginException(PluginException.UNAVAILABLE)

        if (sequence != launchSequence) throw PluginException(PluginException.INVALID_ARGUMENT)
        AlarmRegistry.removeFired(application, sequence)
        launchSequence = 0L
        owner.finish()

        return byteArrayOf()
    }

    /** Called on the main thread after an alarm fires. */
    internal fun wake() {
        val waiter = waiting ?: return
        val record = AlarmRegistry.state(application).after(waiter.cursor, 1, launchSequence).firstOrNull() ?: return
        waiting = null

        if (waiter.continuation.isActive) waiter.continuation.resume(encodeFired(record))
    }
}

/**
 * The context a headless guest's plugins receive: the application, plus the
 * alarm that started the run and a way to end it.
 */
class AlarmLaunchContext internal constructor(
    base: Context,
    internal val record: FiredRecord,
) : ContextWrapper(base) {
    internal var onFinish: (() -> Unit)? = null

    internal fun finish() {
        onFinish?.invoke()
        onFinish = null
    }
}

/** The process-wide store, the live plugin instances, and the receivers' entry points. */
internal object AlarmRegistry {
    private var state: AlarmState? = null
    private var file: AlarmFile? = null
    private val live = mutableSetOf<AndroidAlarmManagerPlugin>()
    private val main = android.os.Handler(Looper.getMainLooper())

    /** Whether a guest is running that will see a fired alarm. */
    val hasLiveGuest get() = live.isNotEmpty()

    fun attach(plugin: AndroidAlarmManagerPlugin) {
        checkMain()
        live.add(plugin)
    }

    fun detach(plugin: AndroidAlarmManagerPlugin) {
        checkMain()
        live.remove(plugin)
    }

    fun onMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else main.post(block)
    }

    fun state(context: Context): AlarmState {
        checkMain()
        state?.let { return it }
        val store = AlarmFile(storeFile(context))
        file = store
        val loaded =
            try {
                store.load()
            } catch (error: IOException) {
                Log.e(TAG, "The alarm store is damaged; starting empty", error)
                AlarmState()
            }
        state = loaded

        return loaded
    }

    /** For tests: forget the in-memory store so the next call reads the file. */
    fun reload() {
        checkMain()
        state = null
        file = null
    }

    fun storeFile(context: Context): File = File(context.noBackupFilesDir, "leptoskit-alarms/alarms.bin")

    /** Applies [change] and saves it; a failed save puts the store back as it was. */
    private fun <T> mutate(
        context: Context,
        change: (AlarmState) -> T,
    ): T {
        val current = state(context)
        val before = current.copy()
        val result = change(current)
        try {
            checkNotNull(file).save(current)
        } catch (error: IOException) {
            state = before
            throw PluginException(PluginException.NATIVE, error)
        }

        return result
    }

    fun schedule(
        context: Context,
        request: AlarmRequest,
        now: Long,
    ): StoredAlarm {
        val current = state(context)

        if (!current.fits(request.id)) throw PluginException(PluginException.BUSY)
        val previous = current.alarm(request.id)
        val pending = StoredAlarm(request.id, request.interval, request.firstFire(now), request.flags, TIMING_INEXACT, request.params)
        val alarm = pending.with(timing = AlarmScheduler.arm(context, pending))
        try {
            mutate(context) { it.put(alarm) }
        } catch (error: PluginException) {
            // The store still holds the previous alarm; put Android back in step with it.
            if (previous == null) AlarmScheduler.disarm(context, alarm.id) else rearm(context, state(context), previous)
            throw error
        }

        return alarm
    }

    fun cancel(
        context: Context,
        id: Int,
    ): Boolean {
        AlarmScheduler.disarm(context, id)

        return mutate(context) { it.remove(id) }
    }

    fun acknowledge(
        context: Context,
        cursor: Long,
        keep: Long,
    ) {
        if (state(context).fired.none { it.sequence <= cursor && it.sequence != keep }) return
        mutate(context) { it.acknowledge(cursor, keep) }
    }

    fun removeFired(
        context: Context,
        sequence: Long,
    ) {
        if (state(context).fired.none { it.sequence == sequence }) return
        mutate(context) { it.removeFired(sequence) }
    }

    /**
     * Records that alarm [id] fired, moves a periodic alarm to its next
     * firing, and wakes the live guests. Returns the record and the alarm's
     * flags, or null when the alarm is no longer scheduled.
     */
    fun fire(
        context: Context,
        id: Int,
        now: Long,
    ): Pair<FiredRecord, Int>? {
        val current = state(context)
        val alarm = current.alarm(id) ?: return null
        val record = current.fire(id, now) ?: return null
        current.alarm(id)?.let { next -> rearm(context, current, next) }
        save(current)

        for (plugin in live.toList()) plugin.wake()

        return record to alarm.flags
    }

    /**
     * Arms the stored alarms again: after a reboot only those marked
     * [RESCHEDULE_ON_REBOOT], forgetting the others, and every one
     * when the exact-alarm permission is granted.
     */
    fun restore(
        context: Context,
        afterBoot: Boolean,
    ) {
        val current = state(context)

        if (afterBoot) current.retain { it.has(RESCHEDULE_ON_REBOOT) }

        for (alarm in current.alarms) rearm(context, current, alarm)
        save(current)
    }

    private fun rearm(
        context: Context,
        current: AlarmState,
        alarm: StoredAlarm,
    ) {
        try {
            current.put(alarm.with(timing = AlarmScheduler.arm(context, alarm)))
        } catch (error: PluginException) {
            // Kept: granting the exact-alarm permission arms it again.
            Log.w(TAG, "Cannot arm alarm ${alarm.id}: ${error.code}")
        }
    }

    /** Receivers keep going when the disk fails; the change stays in memory. */
    private fun save(current: AlarmState) {
        try {
            checkNotNull(file).save(current)
        } catch (error: IOException) {
            Log.e(TAG, "Cannot save the alarm store", error)
        }
    }

    private fun checkMain() {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Alarms run on the main thread" }
    }
}

/** Hands alarms to `AlarmManager`. */
internal object AlarmScheduler {
    fun canScheduleExact(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return true
        val manager = context.getSystemService(AlarmManager::class.java) ?: return false

        return manager.canScheduleExactAlarms()
    }

    /** Arms [alarm] at its next firing and returns how Android times it. */
    fun arm(
        context: Context,
        alarm: StoredAlarm,
    ): Int {
        val manager =
            context.getSystemService(AlarmManager::class.java) ?: throw PluginException(PluginException.UNAVAILABLE)
        val operation = operation(context, alarm.id, create = true) ?: throw PluginException(PluginException.NATIVE)
        val timing = chooseTiming(alarm.flags, canScheduleExact(context))
        // An alarm clock always wakes the device, so its fallback does too.
        val type = if (alarm.has(WAKEUP) || alarm.has(ALARM_CLOCK)) AlarmManager.RTC_WAKEUP else AlarmManager.RTC
        val idle = alarm.has(ALLOW_WHILE_IDLE)

        try {
            when (timing) {
                TIMING_ALARM_CLOCK -> {
                    manager.setAlarmClock(AlarmManager.AlarmClockInfo(alarm.next, show(context, alarm.id)), operation)
                }

                TIMING_EXACT -> {
                    if (idle) {
                        manager.setExactAndAllowWhileIdle(type, alarm.next, operation)
                    } else {
                        manager.setExact(type, alarm.next, operation)
                    }
                }

                else -> {
                    if (idle) manager.setAndAllowWhileIdle(type, alarm.next, operation) else manager.set(type, alarm.next, operation)
                }
            }
        } catch (error: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED, error)
        }

        return timing
    }

    fun disarm(
        context: Context,
        id: Int,
    ) {
        val operation = operation(context, id, create = false) ?: return
        context.getSystemService(AlarmManager::class.java)?.cancel(operation)
        operation.cancel()
    }

    /** Whether the broadcast for [id] exists: from [arm] until [disarm]. */
    fun armed(
        context: Context,
        id: Int,
    ): Boolean = operation(context, id, create = false) != null

    /** One broadcast per alarm ID: the request code tells them apart. */
    private fun operation(
        context: Context,
        id: Int,
        create: Boolean,
    ): PendingIntent? {
        val intent = Intent(context, AlarmReceiver::class.java).setAction(ACTION_FIRE).putExtra(EXTRA_ID, id)
        val mode = if (create) PendingIntent.FLAG_UPDATE_CURRENT else PendingIntent.FLAG_NO_CREATE

        return PendingIntent.getBroadcast(context, id, intent, PendingIntent.FLAG_IMMUTABLE or mode)
    }

    /** What tapping the status-bar alarm icon opens: the app. */
    private fun show(
        context: Context,
        id: Int,
    ): PendingIntent {
        val launch =
            context.packageManager.getLaunchIntentForPackage(context.packageName)
                ?: throw PluginException(PluginException.UNAVAILABLE)

        return PendingIntent.getActivity(
            context,
            id,
            launch,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }
}

/**
 * Starts the guest without UI for an alarm that fired while no guest ran.
 * At most one runs at a time; it stops when the guest finishes the alarm or
 * after [HEADLESS_BUDGET_MS]. An alarm the guest doesn't finish stays queued.
 */
internal object AlarmHeadless {
    private var running = false

    /**
     * Calls [done] with null, without starting anything, when a guest already
     * runs or the app takes over-the-air updates. [ignoreLiveGuests] lets
     * tests run beside the instrumentation's own Activity.
     */
    fun start(
        context: Context,
        record: FiredRecord,
        budgetMillis: Long = HEADLESS_BUDGET_MS,
        ignoreLiveGuests: Boolean = false,
        done: (NativeHeadlessOutcome?) -> Unit,
    ) {
        if (running || (AlarmRegistry.hasLiveGuest && !ignoreLiveGuests)) return done(null)
        val wasm =
            try {
                NativeHeadlessGuest.bundledWasm(context)
            } catch (error: Exception) {
                Log.e(TAG, "Cannot read the bundled guest", error)
                null
            } ?: return done(null)
        val launch = AlarmLaunchContext(context, record)
        val guest =
            try {
                NativeHeadlessGuest(launch, wasm) { bridge, dispatcher -> GeneratedPlugins.register(launch, bridge, dispatcher) }
            } catch (error: Exception) {
                Log.e(TAG, "Cannot start a headless guest for alarm ${record.id}", error)
                return done(NativeHeadlessOutcome.FAILED)
            }
        running = true
        launch.onFinish = { guest.stop(NativeHeadlessOutcome.FINISHED) }
        guest.start(budgetMillis) { outcome ->
            running = false
            done(outcome)
        }
    }
}

/** Receives every alarm this plugin arms. */
class AlarmReceiver : BroadcastReceiver() {
    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        if (intent.action != ACTION_FIRE || !intent.hasExtra(EXTRA_ID)) return
        val application = context.applicationContext ?: context
        val (record, flags) = AlarmRegistry.fire(application, intent.getIntExtra(EXTRA_ID, 0), System.currentTimeMillis()) ?: return

        if (flags and HEADLESS == 0) return
        // Keeps the process, and the alarm's wake lock, until the guest stops.
        val pending = goAsync()
        AlarmHeadless.start(application, record) { pending?.finish() }
    }
}

/** Arms the stored alarms again after a reboot or when exact alarms are allowed. */
class AlarmRestoreReceiver : BroadcastReceiver() {
    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        val application = context.applicationContext ?: context

        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED -> AlarmRegistry.restore(application, afterBoot = true)
            AlarmManager.ACTION_SCHEDULE_EXACT_ALARM_PERMISSION_STATE_CHANGED -> AlarmRegistry.restore(application, afterBoot = false)
        }
    }
}
