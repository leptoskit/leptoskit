package dev.leptoskit.mobile

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import dev.leptoskit.bridge.NativeBridge
import dev.leptoskit.bridge.NativeDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Why a headless guest stopped. */
enum class NativeHeadlessOutcome {
    /** Something the guest called, such as a plugin's finish method, asked to stop. */
    FINISHED,

    /** The time budget ran out first. */
    TIMED_OUT,

    /** The guest trapped, or the host could not run it. */
    FAILED,
}

/**
 * Runs the app's guest with no window: for a broadcast receiver or job that
 * must run app code while no Activity exists, such as an alarm firing.
 *
 * It mirrors [LeptosKitView]'s lifecycle without a renderer. The guest
 * starts with the display's size, its render transactions and Rive frames
 * are taken and dropped, and its plugin calls run on the main looper as
 * usual. Plugins see [context]; an Activity-only plugin finds no Activity and
 * reports `Unavailable`. Create, [start] and [stop] it on the main thread.
 * [stop] may be called from inside a plugin call: it only posts the close.
 */
class NativeHeadlessGuest(
    private val context: Context,
    wasm: ByteArray,
    register: (Long, NativeDispatcher) -> Unit,
) {
    companion object {
        /**
         * The store-shipped guest, or null when the app takes over-the-air
         * updates: the update store belongs to the app's window, and a
         * headless guest must run the same code the app runs.
         */
        fun bundledWasm(context: Context): ByteArray? {
            if (!available(context)) return null
            NativeBridge.load()
            val embedded = context.assets.open("app.lkb").use { it.readBytes() }

            return NativeUpdates.entry(embedded)
        }

        /** Whether [bundledWasm] can return a guest, without reading it. */
        fun available(context: Context): Boolean =
            try {
                val configuration = context.assets.open("codepush.json").use { it.readBytes() }
                !JSONObject(configuration.toString(Charsets.UTF_8)).optBoolean("enabled", false)
            } catch (error: java.io.IOException) {
                Log.w("LeptosKit", "No bundled guest for a headless launch", error)
                false
            }
    }

    private val handler = Handler(Looper.getMainLooper())
    private val dispatcher = NativeDispatcher()
    private var app = 0L
    private var guest = 0L
    private val services =
        NativeServiceOwner(
            post = { task -> check(handler.post { task() }) { "Main looper stopped" } },
            poll = NativeApp::pollClose,
            release = NativeApp::releaseClose,
            active = { if (guest != 0L) NativeApp.guestWake(guest) },
        )
    private val jobs = NativePluginJobs({ guest }, dispatcher, handler) { takeRequests() }
    private val lifecycle = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val timeout = Runnable { stop(NativeHeadlessOutcome.TIMED_OUT) }
    private var onStopped: ((NativeHeadlessOutcome) -> Unit)? = null
    private var started = false
    private var stopping = false

    init {
        checkMain()
        NativeBridge.load()
        try {
            app =
                NativeApp.create(
                    wasm,
                    context.filesDir.absolutePath.toByteArray(Charsets.UTF_8),
                    context.cacheDir.absolutePath.toByteArray(Charsets.UTF_8),
                    context,
                )
            check(app != 0L) { "Cannot create application" }
            checkStatus(NativeApp.setTextMeasurer(app, NativeTextMeasurer(context.resources.displayMetrics)))
            checkStatus(NativeApp.activateService(app, services))
            register(NativeApp.plugins(app), dispatcher)
            // No display frame clock: a guest waiting for frames gets Unavailable.
            NativeApp.enableTimers(app)
            guest = NativeApp.guestSpawn(app, GuestNotifier(handler) { drain() })
            check(guest != 0L) { "Cannot start the guest thread" }
        } catch (error: Exception) {
            release()
            throw error
        }
    }

    /**
     * Starts the guest and stops it after [budgetMillis] unless something
     * stops it first. [onStopped] runs once, on the main thread, after the
     * guest and its plugin jobs have shut down.
     */
    fun start(
        budgetMillis: Long,
        onStopped: (NativeHeadlessOutcome) -> Unit,
    ) {
        checkMain()
        check(!started) { "A headless guest starts once" }
        require(budgetMillis > 0) { "The budget must be positive" }
        started = true
        this.onStopped = onStopped
        handler.postDelayed(timeout, budgetMillis)
        val metrics = context.resources.displayMetrics
        val width = (metrics.widthPixels / metrics.density).coerceAtLeast(1f)
        val height = (metrics.heightPixels / metrics.density).coerceAtLeast(1f)

        if (NativeApp.guestStart(guest, width, height) < 0) stop(NativeHeadlessOutcome.FAILED)
    }

    /** Stops the guest on a later main-looper turn. Later calls are ignored. */
    fun stop(outcome: NativeHeadlessOutcome) {
        handler.post { shutDown(outcome) }
    }

    private fun shutDown(outcome: NativeHeadlessOutcome) {
        if (stopping) return
        stopping = true
        handler.removeCallbacks(timeout)
        val joining = release()
        lifecycle.launch {
            var result = outcome
            try {
                joining.joinAll()
                services.awaitClosed()
            } catch (error: Exception) {
                Log.w("LeptosKit", "A headless guest closed abnormally", error)
                result = NativeHeadlessOutcome.FAILED
            }
            onStopped?.invoke(result)
            onStopped = null
        }
    }

    /** The same order as [LeptosKitView.close]: guest thread, plugin jobs, dispatcher, app. */
    private fun release(): List<Job> {
        services.stop()
        val lent = guest
        guest = 0L

        if (lent != 0L && NativeApp.guestStop(lent) != 0) Log.w("LeptosKit", "The headless guest stopped abnormally")
        val owned = app
        app = 0L
        val joining = jobs.close()
        try {
            dispatcher.close()
        } catch (error: Exception) {
            Log.w("LeptosKit", "A plugin failed to close", error)
        } finally {
            if (owned != 0L) services.close(NativeApp.beginClose(owned))
            services.finishAdmission()
        }

        return joining
    }

    /** Takes what the guest posted: cancellations, requests, then drops what it drew. */
    private fun drain() {
        if (guest == 0L) return
        NativeApp.guestAck(guest)

        if (NativeApp.guestFailed(guest)) {
            Log.w("LeptosKit", "A headless guest failed: ${NativeApp.guestError(guest).toString(Charsets.UTF_8)}")
            stop(NativeHeadlessOutcome.FAILED)

            return
        }

        while (true) {
            val cancelled = NativeApp.guestNextCancelled(guest)

            if (cancelled <= 0L) break
            jobs.cancel(cancelled)
        }

        takeRequests()

        while (NativeApp.guestTakeRender(guest).isNotEmpty()) continue
        val frame = LongArray(3)

        while (NativeApp.guestTakeRiveFrame(guest, frame).isNotEmpty()) continue
    }

    private fun takeRequests() {
        if (guest == 0L) return
        val handles = LongArray(2)

        try {
            while (jobs.canTakeRequest) {
                val request = NativeApp.guestNextRequest(guest, handles) ?: return
                jobs.launch(request, NativeRustWorker.adopt(handles[0]) ?: NativeRustAsync.adopt(handles[1]))
            }
        } catch (error: Exception) {
            Log.w("LeptosKit", "A headless guest's plugin call failed to start", error)
            stop(NativeHeadlessOutcome.FAILED)
        }
    }

    private fun checkStatus(status: Int) {
        check(status >= 0) { NativeApp.error(app).toString(Charsets.UTF_8) }
    }

    private fun checkMain() {
        check(Looper.myLooper() == Looper.getMainLooper()) { "A headless guest lives on the main thread" }
    }
}
