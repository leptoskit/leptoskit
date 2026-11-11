package dev.leptoskit.plugins.androidintent

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ResolveInfo
import android.net.Uri
import android.os.Build
import android.os.Bundle
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.net.URISyntaxException
import java.nio.ByteBuffer

/**
 * Launches, broadcasts and resolves guest-described intents. Decoding in
 * [IntentWire] is the trust boundary: a guest can be replaced over the air,
 * so URI grants, `file:` URIs and parcelable extras never reach Android.
 */
class AndroidIntentPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "android_intent"
    override val methods =
        intArrayOf(LAUNCH, LAUNCH_CHOOSER, SEND_BROADCAST, SEND_SERVICE, CAN_RESOLVE_ACTIVITY, RESOLVED_ACTIVITY, PARSE_AND_LAUNCH)
    private var closed = false

    override fun close() {
        closed = true
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)

        return when (method) {
            LAUNCH -> empty { start(build(IntentWire.decode(payload))) }
            LAUNCH_CHOOSER -> empty { launchChooser(IntentWire.decodeChooser(payload)) }
            SEND_BROADCAST -> empty { broadcast(build(IntentWire.decode(payload))) }
            SEND_SERVICE -> empty { service(build(IntentWire.requireExplicit(IntentWire.decode(payload)))) }
            CAN_RESOLVE_ACTIVITY -> IntentWire.booleanReply(resolve(build(IntentWire.decode(payload))) != null)
            RESOLVED_ACTIVITY -> resolvedReply(resolve(build(IntentWire.decode(payload))))
            PARSE_AND_LAUNCH -> empty { startParsed(parse(IntentWire.decodeIntentUri(payload))) }
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    private inline fun empty(block: () -> Unit): ByteArray {
        block()

        return byteArrayOf()
    }

    private fun launchChooser(chooser: ChooserSpec) {
        val title = chooser.title.ifEmpty { null }

        start(Intent.createChooser(build(chooser.intent), title))
    }

    /**
     * A browser-style `intent:` link names a scheme and resolves to an
     * activity whose filter declares [Intent.CATEGORY_BROWSABLE]. An
     * action-only URI such as Settings has no scheme. API 35 marks that
     * Settings activity browsable, so [start] would open it.
     */
    private fun startParsed(intent: Intent) {
        if (intent.data?.scheme.isNullOrEmpty()) throw PluginException(PluginException.UNAVAILABLE)

        val info = resolve(intent) ?: throw PluginException(PluginException.UNAVAILABLE)
        if (info.filter?.hasCategory(Intent.CATEGORY_BROWSABLE) != true) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        start(intent)
    }

    /** Starts from the host activity when there is one, so the target joins the app's task. */
    private fun start(intent: Intent) {
        if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

        try {
            context.startActivity(intent)
        } catch (error: ActivityNotFoundException) {
            throw PluginException(PluginException.UNAVAILABLE, error)
        } catch (error: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED, error)
        }
    }

    private fun broadcast(intent: Intent) {
        try {
            context.sendBroadcast(intent)
        } catch (error: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED, error)
        }
    }

    /** A null component means no service matched; a background start is refused by Android 8+. */
    private fun service(intent: Intent) {
        val started =
            try {
                context.startService(intent)
            } catch (error: SecurityException) {
                throw PluginException(PluginException.PERMISSION_DENIED, error)
            } catch (error: IllegalStateException) {
                throw PluginException(PluginException.UNAVAILABLE, error)
            } catch (error: IllegalArgumentException) {
                throw PluginException(PluginException.INVALID_ARGUMENT, error)
            }

        if (started == null) throw PluginException(PluginException.UNAVAILABLE)
    }

    /** Subject to Android 11 package visibility: apps the manifest's `<queries>` hides resolve to null. */
    private fun resolve(intent: Intent): ResolveInfo? {
        val packages = context.packageManager

        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            packages.resolveActivity(intent, PackageManager.ResolveInfoFlags.of(PackageManager.MATCH_DEFAULT_ONLY.toLong()))
        } else {
            @Suppress("DEPRECATION")
            packages.resolveActivity(intent, PackageManager.MATCH_DEFAULT_ONLY)
        }
    }

    private fun resolvedReply(info: ResolveInfo?): ByteArray {
        val activity = info?.activityInfo ?: return IntentWire.unresolvedReply()
        val label = info.loadLabel(context.packageManager).toString()

        return IntentWire.resolvedReply(activity.packageName, activity.name, label)
    }

    /**
     * Parses like a browser handles an `intent:` link: no component, no
     * selector, no URI grants, no `file:` data, and only activities that
     * declare CATEGORY_BROWSABLE.
     */
    private fun parse(uri: String): Intent {
        // API 35's parseUri drops grant flags, so the request is read from the URI.
        IntentWire.requireFlags(requestedLaunchFlags(uri))

        val intent =
            try {
                Intent.parseUri(uri, Intent.URI_INTENT_SCHEME)
            } catch (error: URISyntaxException) {
                throw PluginException(PluginException.INVALID_ARGUMENT, error)
            } catch (error: IllegalArgumentException) {
                // A malformed launchFlags or extra value surfaces as NumberFormatException.
                throw PluginException(PluginException.INVALID_ARGUMENT, error)
            }

        if (intent.component != null || intent.selector != null) throw PluginException(PluginException.PERMISSION_DENIED)

        IntentWire.requireFlags(intent.flags)

        if (intent.data?.scheme.equals("file", ignoreCase = true)) throw PluginException(PluginException.PERMISSION_DENIED)

        return intent.addCategory(Intent.CATEGORY_BROWSABLE)
    }

    /** Every `launchFlags=` value in an `intent:` URI, or an invalid number. */
    private fun requestedLaunchFlags(uri: String): Int {
        var flags = 0

        for (match in LAUNCH_FLAGS.findAll(uri)) {
            flags =
                flags or
                try {
                    Integer.decode(match.groupValues[1])
                } catch (error: NumberFormatException) {
                    throw PluginException(PluginException.INVALID_ARGUMENT, error)
                }
        }

        return flags
    }

    private fun build(spec: IntentSpec): Intent {
        val intent = Intent()
        spec.action?.let(intent::setAction)
        intent.addFlags(spec.flags)
        spec.categories.forEach(intent::addCategory)
        val data = spec.data?.let(Uri::parse)

        when {
            data != null && spec.type != null -> intent.setDataAndType(data, spec.type)
            data != null -> intent.setData(data)
            spec.type != null -> intent.setType(spec.type)
        }

        spec.packageName?.let { packageName ->
            intent.setPackage(packageName)
            spec.component?.let { intent.setComponent(ComponentName.createRelative(packageName, it)) }
        }

        if (spec.extras.isNotEmpty()) intent.putExtras(bundle(spec.extras))

        return intent
    }

    private fun bundle(entries: List<Pair<String, ExtraValue>>): Bundle =
        Bundle().apply {
            for ((key, value) in entries) put(this, key, value)
        }

    private fun put(
        bundle: Bundle,
        key: String,
        value: ExtraValue,
    ) {
        when (value) {
            is ExtraValue.Bool -> bundle.putBoolean(key, value.value)
            is ExtraValue.IntValue -> bundle.putInt(key, value.value)
            is ExtraValue.LongValue -> bundle.putLong(key, value.value)
            is ExtraValue.DoubleValue -> bundle.putDouble(key, value.value)
            is ExtraValue.Text -> bundle.putString(key, value.value)
            is ExtraValue.Nested -> bundle.putBundle(key, bundle(value.entries))
            is ExtraValue.Bytes -> bundle.putByteArray(key, value.value.toByteArray())
            is ExtraValue.BoolArray -> bundle.putBooleanArray(key, value.value.toBooleanArray())
            is ExtraValue.IntArray -> bundle.putIntArray(key, value.value.toIntArray())
            is ExtraValue.LongArray -> bundle.putLongArray(key, value.value.toLongArray())
            is ExtraValue.DoubleArray -> bundle.putDoubleArray(key, value.value.toDoubleArray())
            is ExtraValue.StringArray -> bundle.putStringArray(key, value.value.toTypedArray())
            is ExtraValue.IntList -> bundle.putIntegerArrayList(key, ArrayList(value.value))
            is ExtraValue.StringList -> bundle.putStringArrayList(key, ArrayList(value.value))
        }
    }

    private companion object {
        const val LAUNCH = 0
        const val LAUNCH_CHOOSER = 1
        const val SEND_BROADCAST = 2
        const val SEND_SERVICE = 3
        const val CAN_RESOLVE_ACTIVITY = 4
        const val RESOLVED_ACTIVITY = 5
        const val PARSE_AND_LAUNCH = 6
        val LAUNCH_FLAGS = Regex("""(?:^|;)launchFlags=([^;]*)""")
    }
}
