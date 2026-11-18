package dev.leptoskit.plugins.appusage

import android.app.Activity
import android.app.AppOpsManager
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Looper
import android.os.Process
import android.provider.Settings
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer

/**
 * App usage through Usage Access, which the app's owner grants on the
 * system's own screen; the plugin never prompts. Reads require the grant and
 * report permission denial otherwise.
 */
class AppUsagePlugin(
    context: Context,
) : NativePlugin {
    override val name = "app-usage"
    override val methods = intArrayOf(USAGE_ACCESS_STATUS, OPEN_SETTINGS, EVENTS, SESSIONS, SUMMARY)

    private val appContext = context.applicationContext
    private val activity = context as? Activity
    private val usage: UsageStatsManager? =
        context.getSystemService(Context.USAGE_STATS_SERVICE) as? UsageStatsManager
    private val labels = mutableMapOf<String, String>()
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "App usage dispatch requires the UI thread" }

        if (closed) throw PluginException(PluginException.CANCELLED)

        return when (method) {
            USAGE_ACCESS_STATUS -> status(payload)
            OPEN_SETTINGS -> openSettings(payload)
            EVENTS -> events(payload)
            SESSIONS -> sessions(payload)
            SUMMARY -> summary(payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    /** Reports the AppOps mode, never prompting or opening a screen. */
    private fun status(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return byteArrayOf(usageAccessStatus().toByte(), 0, 0, 0)
    }

    /**
     * Whether the system's Usage Access screen granted usage data. Device
     * Admin does not: the AppOps mode stays default without this check.
     */
    private fun usageAccessStatus(): Int {
        val mode = appOpsMode()

        if (mode == AppOpsManager.MODE_ALLOWED) return STATUS_GRANTED

        if (mode == AppOpsManager.MODE_DEFAULT &&
            appContext.checkSelfPermission(PACKAGE_USAGE_STATS) == PackageManager.PERMISSION_GRANTED
        ) {
            return STATUS_GRANTED
        }

        return if (mode == AppOpsManager.MODE_ERRORED || mode == AppOpsManager.MODE_DEFAULT) {
            STATUS_DENIED
        } else {
            STATUS_UNKNOWN
        }
    }

    /** The AppOps mode for usage stats, on every supported API. */
    @Suppress("DEPRECATION")
    private fun appOpsMode(): Int {
        val appOps =
            appContext.getSystemService(Context.APP_OPS_SERVICE) as? AppOpsManager
                ?: return AppOpsManager.MODE_ERRORED

        return if (Build.VERSION.SDK_INT >= UNSAFE_CHECK_API) {
            appOps.unsafeCheckOpNoThrow(
                AppOpsManager.OPSTR_GET_USAGE_STATS,
                Process.myUid(),
                appContext.packageName,
            )
        } else {
            appOps.checkOpNoThrow(
                AppOpsManager.OPSTR_GET_USAGE_STATS,
                Process.myUid(),
                appContext.packageName,
            )
        }
    }

    /** Opens the Usage Access screen; the owner grants access there. */
    private fun openSettings(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        val intent = Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS)

        if (activity != null) {
            activity.startActivity(intent)
        } else {
            appContext.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }

        return byteArrayOf()
    }

    /** Typed events inside the range, capped to the caller's max. */
    private fun events(payload: ByteBuffer): ByteArray {
        val query = UsageQuery.decode(payload, hasMax = true)

        return encodeEventBatch(collect(query).take(query.max))
    }

    /** Per-app foreground sessions clipped to the range's edges. */
    private fun sessions(payload: ByteBuffer): ByteArray {
        val query = UsageQuery.decode(payload, hasMax = false)
        val built = buildSessions(collect(query), query.sinceMs, query.untilMs, ::label)

        if (built.size > MAX_SESSIONS) {
            throw PluginException(
                PluginException.TOO_LARGE,
                IllegalStateException("more than $MAX_SESSIONS sessions in the range; narrow it"),
            )
        }

        return encodeSessionBatch(built)
    }

    /** The range's screen time, unlock count and per-app totals. */
    private fun summary(payload: ByteBuffer): ByteArray {
        val query = UsageQuery.decode(payload, hasMax = false)
        val events = collect(query)
        val sessions = buildSessions(events, query.sinceMs, query.untilMs, ::label)
        val (screenOn, unlocks) = screenOnAndUnlocks(events)

        return encodeSummary(screenOn, unlocks, appTotals(sessions))
    }

    /** Every typed event the system logged inside the range. */
    private fun collect(query: UsageQuery): List<TypedEvent> {
        if (usageAccessStatus() != STATUS_GRANTED) throw PluginException(PluginException.PERMISSION_DENIED)

        val manager =
            usage ?: throw PluginException(
                PluginException.UNAVAILABLE,
                IllegalStateException("no UsageStatsManager on this device"),
            )

        return try {
            typedEvents(manager.queryEvents(query.sinceMs, query.untilMs))
        } catch (error: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED, error)
        } catch (error: RuntimeException) {
            throw PluginException(PluginException.NATIVE, error)
        }
    }

    /** The label the launcher shows for one app, cached per package. */
    private fun label(packageName: String): String =
        labels.getOrPut(packageName) {
            try {
                @Suppress("DEPRECATION")
                appContext.packageManager
                    .getApplicationInfo(packageName, 0)
                    .loadLabel(appContext.packageManager)
                    .toString()
            } catch (_: PackageManager.NameNotFoundException) {
                packageName
            }
        }

    override fun close() {
        closed = true
    }

    private companion object {
        /** Methods the Rust crate calls. */
        const val USAGE_ACCESS_STATUS = 0
        const val OPEN_SETTINGS = 1
        const val EVENTS = 2
        const val SESSIONS = 3
        const val SUMMARY = 4

        /** Usage access states, in wire order. */
        const val STATUS_UNKNOWN = 0
        const val STATUS_GRANTED = 1
        const val STATUS_DENIED = 2

        /** The manifest permission Usage Access is granted through. */
        const val PACKAGE_USAGE_STATS = "android.permission.PACKAGE_USAGE_STATS"

        /** unsafeCheckOpNoThrow arrived in Android 10. */
        const val UNSAFE_CHECK_API = 29
    }
}
