package dev.leptoskit.plugins.recorder

import android.Manifest
import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioManager
import android.media.AudioRecordingConfiguration
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import dev.leptoskit.bridge.NativePermissionHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
import java.nio.ByteBuffer
import kotlin.coroutines.resume
import kotlin.math.max

/**
 * Continuous background microphone segments: 16 kHz mono PCM from
 * [android.media.AudioRecord], encoded to AAC-LC `.m4a` on a dedicated
 * thread, rotated at each boundary. Reads never prompt; only
 * [REQUEST_PERMISSION] does. Background recording needs the `background`
 * plugin's microphone-typed foreground service, which Android 14 cannot
 * start from `BOOT_COMPLETED`, so the owner must open the app once after a
 * reboot.
 */
class RecorderPlugin(
    context: Context,
) : NativePlugin {
    override val name = "recorder"
    override val methods =
        intArrayOf(PERMISSION, REQUEST_PERMISSION, START, STOP, STATUS, NEXT_SEGMENT, RECOVER)

    private val appContext = context.applicationContext
    private val activity = context as? Activity
    private val permissionHost = context as? NativePermissionHost
    private val preferences = appContext.getSharedPreferences("leptoskit.recorder", Context.MODE_PRIVATE)
    private val audioManager = appContext.getSystemService(Context.AUDIO_SERVICE) as? AudioManager
    private val listener = EngineListener()
    private var closed = false

    @Volatile
    private var silenced = false

    private val lock = Object()
    private var state = STATE_IDLE
    private var reason: String? = null
    private var segmentStartMs = 0L
    private var engine: RecorderEngine? = null
    private var index: RecorderIndex? = null
    private val undelivered = ArrayDeque<SegmentReply>()
    private var waiter: CancellableContinuation<Unit>? = null

    init {
        trackForeground()
        listenForSilencing()
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Recorder dispatch requires the UI thread" }
        currentCoroutineContext().ensureActive()

        if (closed) throw PluginException(PluginException.CANCELLED)

        return when (method) {
            PERMISSION -> reportedPermission(payload)
            REQUEST_PERMISSION -> requestPermission(payload)
            START -> start(payload)
            STOP -> stop(payload)
            STATUS -> status(payload)
            NEXT_SEGMENT -> nextSegment(payload)
            RECOVER -> recover(payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    /** Reports the microphone permission, without prompting. */
    private fun reportedPermission(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return permissionReply()
    }

    /** Shows the system permission screen once, then reports what it left. */
    private suspend fun requestPermission(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        if (granted()) return permissionReply()

        val host = permissionHost ?: throw PluginException(PluginException.UNAVAILABLE)
        requireDeclared()
        host.requestNativePermissions(arrayOf(Manifest.permission.RECORD_AUDIO))
        return permissionReply()
    }

    /** Starts recording, delivering an earlier session's leftovers first. */
    private fun start(payload: ByteBuffer): ByteArray {
        synchronized(lock) {
            if (state != STATE_IDLE && state != STATE_FAILED) throw PluginException(PluginException.BUSY)
        }

        val request = StartRequest(payload)
        val resolved = resolveDirectory(request.directory)

        if (!granted()) throw PluginException(PluginException.PERMISSION_DENIED)

        if (foregroundActivities == 0 && !microphoneServiceRunning()) {
            Log.w(
                "leptoskit.recorder",
                "start refused: the app is not in the foreground and no microphone-typed " +
                    "foreground service is running; start the background plugin's service first",
            )
            throw PluginException(PluginException.UNAVAILABLE)
        }

        preferences.edit().putString(DIRECTORY_KEY, resolved.path).apply()

        val recovered = recoverInto(resolved)
        val freshIndex = RecorderIndex(resolved)

        synchronized(lock) {
            index = freshIndex
            engine =
                RecorderEngine(
                    resolved,
                    freshIndex,
                    request.sampleRate,
                    request.bitrate,
                    request.segmentSecs.toLong() * request.sampleRate,
                    request.thresholdDbfs,
                    listener,
                ).also { it.start() }
            state = STATE_RECORDING
            reason = null
        }
        return encodeRecovery(recovered.first, recovered.second)
    }

    /** Stops recording and waits for the last segment, so it is delivered. */
    private suspend fun stop(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        val running = synchronized(lock) { engine }
        running?.stop()
        if (running != null) {
            withContext(Dispatchers.Default) { running.join(5_000) }
        }

        synchronized(lock) {
            state = STATE_IDLE
            reason = null
            segmentStartMs = 0
            engine = null
        }
        return byteArrayOf()
    }

    /** Reports the current state, with the open segment's start time. */
    private fun status(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        val snapshot = synchronized(lock) { Triple(state, segmentStartMs, reason) }
        val silencedWhileRecording = snapshot.first == STATE_RECORDING && silenced
        val reported = if (silencedWhileRecording) STATE_SILENCED else snapshot.first

        return encodeStatus(
            reported,
            if (reported == STATE_RECORDING || reported == STATE_SILENCED) snapshot.second else 0,
            if (reported == STATE_FAILED) snapshot.third else null,
        )
    }

    /** Delivers one finished segment, waits for the next, or ends the stream. */
    private suspend fun nextSegment(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        while (true) {
            val ready = synchronized(lock) { undelivered.removeFirstOrNull() }
            if (ready != null) {
                markDelivered(ready)
                return ready.encode()
            }

            val waiting =
                synchronized(lock) {
                    state == STATE_RECORDING || state == STATE_SILENCED || state == STATE_INTERRUPTED
                }
            if (!waiting) return encodeAbsentSegment()

            suspendCancellableCoroutine { continuation ->
                synchronized(lock) {
                    if (undelivered.isNotEmpty()) {
                        continuation.resume(Unit)
                    } else {
                        waiter = continuation
                        continuation.invokeOnCancellation {
                            synchronized(lock) {
                                if (waiter === continuation) waiter = null
                            }
                        }
                    }
                }
            }
        }
    }

    /** Delivers an earlier session's leftovers, without recording. */
    private fun recover(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        synchronized(lock) {
            if (state != STATE_IDLE && state != STATE_FAILED) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        }

        val remembered = preferences.getString(DIRECTORY_KEY, null) ?: return encodeRecovery(0, 0)
        val resolved = File(remembered)
        if (!resolved.isDirectory) return encodeRecovery(0, 0)

        val recovered = recoverInto(resolved)
        return encodeRecovery(recovered.first, recovered.second)
    }

    override fun close() {
        closed = true
        synchronized(lock) {
            engine?.stop()
            engine = null
            state = STATE_IDLE
            waiter?.cancel(PluginException(PluginException.CANCELLED))
            waiter = null
        }
    }

    /** Marks the file behind a delivered segment so recovery skips it. */
    private fun markDelivered(reply: SegmentReply) {
        val index = synchronized(lock) { index } ?: return
        try {
            index.recordDelivered(File(reply.path).name)
        } catch (_: IOException) {
            // Delivery already happened; a later recovery may repeat it.
        }
    }

    /** Scans a directory for leftovers, re-queueing and deleting as found. */
    private fun recoverInto(resolved: File): Pair<Int, Int> {
        // A fresh journal, not the running session's, so recovery always
        // reads the directory it was asked about.
        val journal = RecorderIndex(resolved)
        val plan =
            RecorderIndex.plan(
                journal.read(),
                resolved.listFiles()?.map { it.name } ?: emptyList(),
            )

        synchronized(lock) {
            for (record in plan.deliverable) {
                undelivered.add(
                    SegmentReply(
                        File(resolved, record.filename).absolutePath,
                        record.startedAtMs,
                        record.durationMs,
                        max(record.bytes, 1),
                        record.peakDbfs,
                        record.activeMs,
                        ENDED_RECOVERED,
                    ),
                )
            }
        }

        for (name in plan.discardable) {
            // A truncated file cannot be read; delete it and report it once.
            File(resolved, name).delete()
        }

        journal.compact(plan.deliverable)
        return plan.deliverable.size to plan.discardable.size
    }

    /** The permission state the wire reports. */
    private fun permissionReply(): ByteArray {
        val denied = activity?.shouldShowRequestPermissionRationale(Manifest.permission.RECORD_AUDIO) == true

        return encodePermission(
            when {
                granted() -> PERMISSION_GRANTED
                denied -> PERMISSION_DENIED
                else -> PERMISSION_UNKNOWN
            },
        )
    }

    private fun granted(): Boolean = appContext.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    /** Asking for an undeclared permission is a caller error. */
    @Suppress("DEPRECATION")
    private fun requireDeclared() {
        val declared =
            appContext.packageManager
                .getPackageInfo(appContext.packageName, PackageManager.GET_PERMISSIONS)
                .requestedPermissions
                .orEmpty()

        if (Manifest.permission.RECORD_AUDIO !in declared) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    /** An app-private directory inside the app's own data or cache dirs. */
    private fun resolveDirectory(path: String): File {
        val resolved =
            try {
                File(path).canonicalFile
            } catch (_: IOException) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }

        val roots =
            listOfNotNull(
                appContext.filesDir,
                appContext.cacheDir,
                if (Build.VERSION.SDK_INT >= 24) appContext.dataDir else null,
                appContext.noBackupFilesDir,
            ).mapNotNull { root ->
                try {
                    root.canonicalFile
                } catch (_: IOException) {
                    null
                }
            }

        val inside = roots.any { root -> resolved == root || resolved.path.startsWith(root.path + "/") }
        if (!inside) throw PluginException(PluginException.INVALID_ARGUMENT)
        if (!resolved.exists() && !resolved.mkdirs()) throw PluginException(PluginException.INVALID_ARGUMENT)
        if (resolved.exists() && !resolved.isDirectory) throw PluginException(PluginException.INVALID_ARGUMENT)

        return resolved
    }

    /** Counts started activities, so a foreground start can be told apart. */
    private fun trackForeground() {
        val app = appContext as? Application ?: return

        synchronized(Companion) {
            if (foregroundTracked) return
            foregroundTracked = true
            app.registerActivityLifecycleCallbacks(
                object : Application.ActivityLifecycleCallbacks {
                    override fun onActivityStarted(activity: Activity) {
                        foregroundActivities += 1
                    }

                    override fun onActivityStopped(activity: Activity) {
                        foregroundActivities -= 1
                    }

                    override fun onActivityCreated(
                        activity: Activity,
                        savedInstanceState: android.os.Bundle?,
                    ) {}

                    override fun onActivityResumed(activity: Activity) {}

                    override fun onActivityPaused(activity: Activity) {}

                    override fun onActivitySaveInstanceState(
                        activity: Activity,
                        outState: android.os.Bundle,
                    ) {}

                    override fun onActivityDestroyed(activity: Activity) {}
                },
            )
        }
    }

    /** Android silences our capture when another app needs the microphone. */
    private fun listenForSilencing() {
        if (Build.VERSION.SDK_INT < 29) return

        audioManager?.registerAudioRecordingCallback(
            object : AudioManager.AudioRecordingCallback() {
                override fun onRecordingConfigChanged(configs: List<AudioRecordingConfiguration>) {
                    silenced = configs.any { it.isClientSilenced() }
                }
            },
            Handler(Looper.getMainLooper()),
        )
    }

    /**
     * Whether the `background` plugin's foreground service runs with the
     * microphone type. Read reflectively so apps that only record in the
     * foreground do not need that plugin compiled in.
     */
    private fun microphoneServiceRunning(): Boolean =
        try {
            val state = Class.forName("dev.leptoskit.plugins.background.BackgroundState")
            val running = state.getField("running").getBoolean(null)
            val types = state.getField("serviceTypes").getInt(null)
            running && (types and MIC_SERVICE_TYPE) != 0
        } catch (_: Throwable) {
            false
        }

    /** Forwards engine events into the delivery queue and status. */
    private inner class EngineListener : RecorderEngine.Listener {
        override fun onSegment(
            path: String,
            startedAtMs: Long,
            durationMs: Long,
            bytes: Long,
            peakDbfs: Double,
            activeMs: Long,
            endedBy: Int,
        ) {
            val wake: CancellableContinuation<Unit>? =
                synchronized(lock) {
                    undelivered.add(
                        SegmentReply(path, startedAtMs, durationMs, bytes, peakDbfs, activeMs, endedBy),
                    )
                    val waiting = waiter
                    waiter = null
                    waiting
                }

            if (wake != null && !wake.isCancelled) wake.resume(Unit)
        }

        override fun onOpened(startedAtMs: Long) {
            synchronized(lock) {
                if (state == STATE_RECORDING || state == STATE_SILENCED) segmentStartMs = startedAtMs
            }
        }

        override fun onFailure(failure: String) {
            synchronized(lock) {
                state = STATE_FAILED
                reason = failure
                segmentStartMs = 0
                engine = null
            }
        }
    }

    companion object {
        const val PERMISSION = 0
        const val REQUEST_PERMISSION = 1
        const val START = 2
        const val STOP = 3
        const val STATUS = 4
        const val NEXT_SEGMENT = 5
        const val RECOVER = 6

        /** The background plugin's microphone foreground-service type bit. */
        private const val MIC_SERVICE_TYPE = 4

        private const val DIRECTORY_KEY = "directory"

        @Volatile
        private var foregroundActivities = 0

        @Volatile
        private var foregroundTracked = false
    }
}
