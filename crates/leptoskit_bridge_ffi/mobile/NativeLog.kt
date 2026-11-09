package dev.leptoskit.mobile

import android.util.Log

/**
 * The Android host's records, written through leptoskit's logger so they reach
 * logcat at the runtime's level and under the same `leptoskit/<part>` tags.
 * Before the native library loads, they go to logcat directly.
 *
 * A failure's record carries its whole cause chain: each cause's message and
 * stack, and what it suppressed.
 */
internal object NativeLog {
    const val BRIDGE = "leptoskit/bridge"
    const val GPU = "leptoskit/gpu"
    const val RENDER = "leptoskit/render"

    fun info(
        tag: String,
        message: String,
    ) = write(INFO, tag, message)

    fun warn(
        tag: String,
        message: String,
        error: Throwable? = null,
    ) = write(WARN, tag, describe(message, error))

    fun error(
        tag: String,
        message: String,
        error: Throwable? = null,
    ) = write(ERROR, tag, describe(message, error))

    /** [message], then [error]'s stack and every cause and suppressed failure under it. */
    fun describe(
        message: String,
        error: Throwable?,
    ): String = if (error == null) message else "$message: ${Log.getStackTraceString(error).trimEnd()}"

    private fun write(
        level: Int,
        tag: String,
        message: String,
    ) {
        try {
            NativeApp.log(level, tag.toByteArray(Charsets.UTF_8), message.toByteArray(Charsets.UTF_8))
        } catch (_: UnsatisfiedLinkError) {
            val priority =
                when (level) {
                    ERROR -> Log.ERROR
                    WARN -> Log.WARN
                    else -> Log.INFO
                }
            Log.println(priority, tag, message)
        }
    }

    private const val ERROR = 1
    private const val WARN = 2
    private const val INFO = 3
}
