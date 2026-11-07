package dev.leptoskit.plugins.device

import android.app.ActivityManager
import android.app.UiModeManager
import android.content.Context
import android.content.res.Configuration
import android.os.Build
import android.os.Environment
import android.os.StatFs
import android.provider.Settings
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.nio.ByteBuffer

/**
 * Device information without persistent identifiers (method 0) and, only on
 * request, the user-assigned device name (method 1). The byte layout is
 * documented in the Rust crate and mirrored by [DeviceWire].
 */
class DevicePlugin internal constructor(
    private val readFacts: () -> DeviceFacts,
    private val readIdentifiers: () -> DeviceIdentifiers,
) : NativePlugin {
    constructor(context: Context) : this(
        { readFacts(context) },
        { readIdentifiers(context) },
    )

    override val name = "device"
    override val methods = intArrayOf(SNAPSHOT, IDENTIFIERS)

    private companion object {
        /** Device facts without identifiers. */
        const val SNAPSHOT = 0

        /** The device name; Android gives apps no other identifier here. */
        const val IDENTIFIERS = 1
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (method != SNAPSHOT && method != IDENTIFIERS) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        // Feature lists and volume statistics cross into system services and
        // the file system, so read them off the UI thread.
        return withContext(Dispatchers.IO) {
            if (method == SNAPSHOT) DeviceWire.snapshot(readFacts()) else DeviceWire.identifiers(readIdentifiers())
        }
    }
}

/** Treats blank values and `Build.UNKNOWN` as unknown. */
private fun known(value: String?): String? = DeviceWire.known(value)

/** The smallest-width threshold Android's own resources use for tablets. */
private const val TABLET_WIDTH_DP = 600

/** Reads this device. Nothing here is a persistent identifier. */
internal fun readFacts(context: Context): DeviceFacts {
    val activity = context.getSystemService(Context.ACTIVITY_SERVICE) as? ActivityManager
    val memory = ActivityManager.MemoryInfo().also { activity?.getMemoryInfo(it) }
    val volume = runCatching { StatFs(Environment.getDataDirectory().path) }.getOrNull()

    return DeviceFacts(
        kind = deviceKind(context),
        simulator = if (Build.HARDWARE == "goldfish" || Build.HARDWARE == "ranchu") true else null,
        totalMemory = if (activity == null) 0 else memory.totalMem.coerceAtLeast(0),
        availableMemory = if (activity == null) null else memory.availMem.coerceAtLeast(0),
        totalDisk = volume?.totalBytes ?: 0,
        freeDisk = volume?.availableBytes,
        cpuCount = Runtime.getRuntime().availableProcessors(),
        manufacturer = known(Build.MANUFACTURER),
        brand = known(Build.BRAND),
        model = known(Build.MODEL),
        osName = "Android",
        osVersion = known(Build.VERSION.RELEASE),
        architecture = known(Build.SUPPORTED_ABIS.firstOrNull()),
        kernelVersion = known(System.getProperty("os.version")),
        build = readBuild(context, activity, memory),
    )
}

private fun deviceKind(context: Context): Int {
    val mode = context.getSystemService(Context.UI_MODE_SERVICE) as? UiModeManager

    return when (mode?.currentModeType) {
        Configuration.UI_MODE_TYPE_TELEVISION -> {
            3
        }

        Configuration.UI_MODE_TYPE_VR_HEADSET -> {
            7
        }

        Configuration.UI_MODE_TYPE_WATCH -> {
            5
        }

        Configuration.UI_MODE_TYPE_CAR -> {
            6
        }

        // A desk dock holds a phone or tablet; it does not make one a desktop.
        Configuration.UI_MODE_TYPE_NORMAL, Configuration.UI_MODE_TYPE_DESK -> {
            val width = context.resources.configuration.smallestScreenWidthDp
            when {
                width >= TABLET_WIDTH_DP -> 2
                width > 0 -> 1
                else -> 0
            }
        }

        else -> {
            0
        }
    }
}

/** `Build` and `ActivityManager` details for the Android section. */
private fun readBuild(
    context: Context,
    activity: ActivityManager?,
    memory: ActivityManager.MemoryInfo,
): AndroidBuild =
    AndroidBuild(
        sdkInt = Build.VERSION.SDK_INT,
        previewSdkInt = Build.VERSION.PREVIEW_SDK_INT,
        lowRamDevice = activity?.isLowRamDevice,
        lowMemory = if (activity == null) null else memory.lowMemory,
        buildTime = Build.TIME.takeIf { it > 0 },
        baseOs = known(Build.VERSION.BASE_OS),
        codename = known(Build.VERSION.CODENAME),
        incremental = known(Build.VERSION.INCREMENTAL),
        release = known(Build.VERSION.RELEASE),
        securityPatch = known(Build.VERSION.SECURITY_PATCH),
        board = known(Build.BOARD),
        bootloader = known(Build.BOOTLOADER),
        device = known(Build.DEVICE),
        display = known(Build.DISPLAY),
        fingerprint = known(Build.FINGERPRINT),
        hardware = known(Build.HARDWARE),
        host = known(Build.HOST),
        id = known(Build.ID),
        product = known(Build.PRODUCT),
        tags = known(Build.TAGS),
        type = known(Build.TYPE),
        supportedAbis = DeviceWire.shortList(Build.SUPPORTED_ABIS.asList()),
        supported32BitAbis = DeviceWire.shortList(Build.SUPPORTED_32_BIT_ABIS.asList()),
        supported64BitAbis = DeviceWire.shortList(Build.SUPPORTED_64_BIT_ABIS.asList()),
        systemFeatures = DeviceWire.features(context.packageManager.systemAvailableFeatures.mapNotNull { it.name }),
    )

/**
 * `Settings.Global.DEVICE_NAME`, the name shown in Bluetooth and About phone.
 * Android gives ordinary apps no vendor or machine identifier here: the serial
 * needs a privileged permission.
 */
internal fun readIdentifiers(context: Context): DeviceIdentifiers =
    DeviceIdentifiers(
        deviceName =
            DeviceWire.boundedIdentifier(
                Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME),
            ),
    )
