package dev.leptoskit.mobile

import android.content.Context
import android.content.res.Configuration
import android.os.Handler
import android.os.Looper
import android.view.Choreographer
import android.widget.FrameLayout
import android.widget.TextView
import dev.leptoskit.bridge.NativeBridge
import dev.leptoskit.bridge.NativeDispatcher
import kotlinx.coroutines.Job
import kotlinx.coroutines.joinAll
import java.nio.ByteBuffer

/** [NativeApp.guestTakeSystemBack]'s answers, as `lk_guest_take_system_back` defines them. */
private const val SYSTEM_BACK_NONE = 0
private const val SYSTEM_BACK_WENT_BACK = 1
private const val SYSTEM_BACK_LEAVE = 2

internal class NativeApplicationInitializationFailure(
    cause: Exception,
    val join: suspend () -> Unit,
) : IllegalStateException(cause.message, cause)

/**
 * Called by JNI on the guest thread after it posts something; the view takes
 * it on the main looper.
 */
internal class GuestNotifier(
    private val handler: Handler,
    private val drain: Runnable,
) {
    @Suppress("unused") // Called by JNI.
    fun notified() {
        handler.post(drain)
    }
}

/**
 * UIKit's counterpart: real Android controls driven by the Rust renderer. The
 * guest runs on a thread of its own, so slow app logic never stops scrolling
 * or touch handling here; this view posts its inputs to the guest and applies
 * what it posts back.
 */
class LeptosKitView(
    context: Context,
    wasm: ByteArray,
    private val onReady: () -> Unit = {},
    private val onFailure: ((Exception) -> Unit)? = null,
    register: (Long, NativeDispatcher) -> Unit,
) : FrameLayout(context),
    AutoCloseable {
    private var app = 0L
    private var guest = 0L
    private var riveAnimator: NativeRiveAnimator? = null
    private val dispatcher = NativeDispatcher()
    private val handler = Handler(Looper.getMainLooper())
    private val services =
        handler.let { main ->
            NativeServiceOwner(
                post = { task -> check(main.post { task() }) { "Main looper stopped" } },
                poll = NativeApp::pollClose,
                release = NativeApp::releaseClose,
                active = { if (guest != 0L) NativeApp.guestWake(guest) },
            )
        }
    private val jobs = NativePluginJobs({ guest }, dispatcher, handler) { runUi { takeRequests() } }
    private var closingJobs: List<Job> = emptyList()
    private var closeFailure: Exception? = null
    private var closeStarted = false
    private var frameScheduled = false

    /** Answers owed for system Back offers the guest has not answered yet, oldest first. */
    private val systemBackAnswers = ArrayDeque<(Boolean) -> Unit>()

    /** The guest's frame clock: a display frame, posted only while the guest waits for one. */
    private val frameCallback = Choreographer.FrameCallback { frameTimeNanos -> deliverFrame(frameTimeNanos) }
    private val renderer = NativeWidgetRenderer(this, ::event) { onCanvasChanged?.invoke(this) }

    /** The root's opaque background, for the host to paint behind system bars. */
    val canvas: Int? get() = renderer.canvas

    /** Called on the UI thread when [canvas] changes. */
    var onCanvasChanged: ((LeptosKitView) -> Unit)? = null
    private var started = false
    private var ready = false
    private var readyScheduled = false
    private val density get() = resources.displayMetrics.density

    init {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Application creation requires the main thread" }
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
            checkStatus(NativeApp.setTextMeasurer(app, NativeTextMeasurer(resources.displayMetrics)))
            checkStatus(NativeApp.activateService(app, services))
            register(NativeApp.plugins(app), dispatcher)
            // This view drives the guest's frame clock from the Choreographer.
            checkStatus(NativeApp.enableFrames(app))
            // Timers are optional: an unavailable one stays `Unavailable` to
            // the guest instead of failing application startup.
            NativeApp.enableTimers(app)
            // 3D scenes, when the app was built with them.
            NativeApp.enableScene3d(app)
            guest = NativeApp.guestSpawn(app, GuestNotifier(handler) { runUi { drain() } })
            check(guest != 0L) { "Cannot start the guest thread" }
            // A background plugin wakes the service through this host hook.
            NativeServiceWakeBroker.install { if (guest != 0L) NativeApp.guestWakeBackground(guest) }
            riveAnimator =
                NativeRiveAnimator(
                    this,
                    hasScenes = { guest != 0L && NativeApp.guestHasRive(guest) },
                    // The guest advances the scenes; their frames come back through drain.
                    tick = { elapsed, scale, flags ->
                        if (guest == 0L) {
                            false
                        } else {
                            val status = NativeApp.guestRiveTick(guest, elapsed, scale, flags, renderer.visibleRiveNodes())
                            check(status >= 0) { "Invalid native Rive display environment" }
                            status == 1
                        }
                    },
                    fail = { error -> runUi { throw error } },
                )
            renderer.onRiveViewportChange = { riveAnimator?.requestFrame() }
            // Creating the app starts its DevTools report when `leptoskit dev` named a hub.
            renderer.attachDevtools()
        } catch (error: Exception) {
            NativeLog.error(NativeLog.BRIDGE, "cannot start the app", error)
            close()
            throw NativeApplicationInitializationFailure(error) { closeAndJoin() }
        }
    }

    /** Stop admission before transferring App ownership; plugin joins remain awaitable. */
    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Application close requires the main thread" }

        if (closeStarted) return
        closeStarted = true
        NativeServiceWakeBroker.install(null)
        riveAnimator?.close()
        Choreographer.getInstance().removeFrameCallback(frameCallback)
        frameScheduled = false
        // The guest that could have gone back is gone; see offerSystemBack.
        systemBackAnswers.clear()
        services.stop()
        // Waits only for the guest's step in progress, then the app is ours to close.
        val lent = guest
        guest = 0L

        if (lent != 0L && NativeApp.guestStop(lent) != 0) NativeLog.warn(NativeLog.BRIDGE, "the guest thread stopped abnormally")
        val owned = app
        app = 0L
        closingJobs = jobs.close()
        try {
            dispatcher.close()
        } catch (error: Exception) {
            closeFailure = error
        } finally {
            if (owned != 0L) services.close(NativeApp.beginClose(owned))
            services.finishAdmission()
            renderer.clear()
        }
    }

    /** Cancelling this await never cancels the retained native close owner. */
    suspend fun closeAndJoin() {
        close()
        closingJobs.joinAll()
        services.awaitClosed()
        closeFailure?.let { throw it }
    }

    /**
     * Offers Android system Back to the guest's optional `leptoskit_system_back`
     * export, which pops the file router when it can. The guest answers on its
     * own thread, so [answer] runs later, on the main thread: true when the
     * guest went back, false when the activity should leave because there was
     * nothing to go back to or the guest does not export `leptoskit_system_back`.
     * Without a running guest, [answer] runs at once with false.
     *
     * Offers are answered in order, so a second Back while the guest is still
     * answering the first waits its turn, as a second press would. An offer
     * still unanswered when this view closes is never answered: the guest that
     * could have gone back is gone, and the next Back reaches whatever replaced it.
     */
    fun offerSystemBack(answer: (Boolean) -> Unit) {
        check(Looper.myLooper() == Looper.getMainLooper()) { "System Back requires the main thread" }

        if (guest == 0L || NativeApp.guestSystemBack(guest) < 0) {
            answer(false)

            return
        }

        systemBackAnswers.addLast(answer)
    }

    override fun dispatchTouchEvent(event: android.view.MotionEvent): Boolean {
        dismissEditorOutside(this, event)

        return super.dispatchTouchEvent(event)
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        riveAnimator?.requestFrame()
    }

    override fun onDetachedFromWindow() {
        riveAnimator?.detach()
        super.onDetachedFromWindow()
    }

    override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
        super.onWindowFocusChanged(hasWindowFocus)
        riveAnimator?.requestFrame()
    }

    override fun onWindowVisibilityChanged(visibility: Int) {
        super.onWindowVisibilityChanged(visibility)
        riveAnimator?.requestFrame()
    }

    override fun onVisibilityChanged(
        changedView: android.view.View,
        visibility: Int,
    ) {
        super.onVisibilityChanged(changedView, visibility)
        riveAnimator?.requestFrame()
    }

    override fun onSizeChanged(
        width: Int,
        height: Int,
        oldWidth: Int,
        oldHeight: Int,
    ) {
        super.onSizeChanged(width, height, oldWidth, oldHeight)

        if (guest == 0L || width <= 0 || height <= 0) return
        // Adding children during the current layout pass can discard their
        // requestLayout. Resize on the next UI turn so Android measures them.
        post {
            if (guest == 0L) return@post
            runUi { resizeGuest(width, height) }
        }
    }

    override fun onConfigurationChanged(configuration: Configuration) {
        super.onConfigurationChanged(configuration)
        // Density and display preferences can change without a pixel-size change.
        post {
            if (guest != 0L && width > 0 && height > 0) runUi { resizeGuest(width, height) }
        }
    }

    private fun resizeGuest(
        width: Int,
        height: Int,
    ) {
        if (!started) {
            started = true
            checkGuest(NativeApp.guestStart(guest, width / density, height / density))
        } else {
            val dark =
                resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK ==
                    Configuration.UI_MODE_NIGHT_YES
            checkGuest(NativeApp.guestResize(guest, width / density, height / density, dark))
        }
    }

    private fun checkStatus(status: Int) {
        check(status >= 0) { NativeApp.error(app).toString(Charsets.UTF_8) }
    }

    private fun checkGuest(status: Int) {
        check(status >= 0) { NativeApp.guestError(guest).toString(Charsets.UTF_8).ifEmpty { "Application closed" } }
    }

    private fun runUi(block: () -> Unit) {
        try {
            block()
        } catch (error: Exception) {
            // Every build logs why the app stopped drawing, with its causes.
            NativeLog.error(NativeLog.BRIDGE, "the app stopped", error)
            close()
            removeAllViews()

            if (onFailure != null) {
                onFailure.invoke(error)

                return
            }

            addView(
                TextView(context).apply {
                    text = error.message ?: error.javaClass.simpleName
                    contentDescription = "leptoskit-error"
                },
            )
        }
    }

    /**
     * Takes everything the guest posted: cancellations before new requests,
     * requests the job budget admits, transactions in order, and Rive frames.
     */
    private fun drain() {
        if (guest == 0L) return
        NativeApp.guestAck(guest)
        check(!NativeApp.guestFailed(guest)) { NativeApp.guestError(guest).toString(Charsets.UTF_8) }

        while (true) {
            val cancelled = NativeApp.guestNextCancelled(guest)

            if (cancelled == 0L) break
            check(cancelled > 0) { "Invalid cancellation notification" }
            jobs.cancel(cancelled)
        }

        takeRequests()

        while (true) {
            val transaction = NativeApp.guestTakeRender(guest)

            if (transaction.isEmpty()) break
            renderer.render(RenderBatch.fromBinary(transaction))
        }

        presentRiveFrames()
        riveAnimator?.requestFrame()
        scheduleFrame()

        if (!ready && !readyScheduled && childCount > 0) {
            readyScheduled = true
            post { reportReady() }
        }

        answerSystemBack()
    }

    private fun takeRequests() {
        if (guest == 0L) return
        val handles = LongArray(2)

        while (jobs.canTakeRequest) {
            val request = NativeApp.guestNextRequest(guest, handles) ?: return
            jobs.launch(request, NativeRustWorker.adopt(handles[0]) ?: NativeRustAsync.adopt(handles[1]))
        }
    }

    /**
     * Answers system Back offers in order, last in a drain: the screen they
     * changed is applied, and an answer that leaves the activity, or closes
     * this view, has nothing of the drain left to run after it.
     */
    private fun answerSystemBack() {
        while (guest != 0L) {
            val wentBack =
                when (NativeApp.guestTakeSystemBack(guest)) {
                    SYSTEM_BACK_NONE -> return
                    SYSTEM_BACK_WENT_BACK -> true
                    SYSTEM_BACK_LEAVE -> false
                    else -> error("Invalid system Back answer")
                }
            val answer = systemBackAnswers.removeFirstOrNull() ?: error("System Back answered without an offer")
            answer(wentBack)
        }
    }

    private fun presentRiveFrames() {
        val frame = LongArray(3)

        while (true) {
            val pixels = NativeApp.guestTakeRiveFrame(guest, frame)

            if (pixels.isEmpty()) return
            check(renderer.presentRive(frame[0], frame[1].toInt(), frame[2].toInt(), ByteBuffer.wrap(pixels))) {
                "Cannot present a Rive frame"
            }
        }
    }

    private fun event(
        node: Long,
        kind: Int,
        value: String = "",
    ) {
        if (guest == 0L || renderer.applying) return
        runUi { checkGuest(NativeApp.guestEvent(guest, node, kind, value.toByteArray(Charsets.UTF_8))) }
    }

    private fun scheduleFrame() {
        if (guest == 0L || frameScheduled || NativeApp.guestFrameRequested(guest) != 1) return
        frameScheduled = true
        Choreographer.getInstance().postFrameCallback(frameCallback)
    }

    /** Posts the frame to the guest, and keeps the Choreographer running while it waits for frames. */
    private fun deliverFrame(frameTimeNanos: Long) {
        frameScheduled = false

        if (guest == 0L) return
        val refreshRate = display?.refreshRate?.takeIf { it > 0f } ?: 60f
        runUi {
            checkGuest(NativeApp.guestFrame(guest, frameTimeNanos, (1_000_000_000f / refreshRate).toLong()))
            scheduleFrame()
        }
    }

    private fun reportReady() {
        readyScheduled = false

        if (app != 0L && !ready && isAttachedToWindow && childCount > 0) {
            ready = true
            onReady()
        }
    }
}
