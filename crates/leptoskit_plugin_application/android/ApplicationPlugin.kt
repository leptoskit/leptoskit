package dev.leptoskit.plugins.application

import android.content.Context
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.os.Build
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer

/**
 * The installed application's identity, signing certificate, install source
 * and install times. The byte layout is documented in the Rust crate and
 * mirrored by [ApplicationWire].
 */
class ApplicationPlugin internal constructor(
    private val readFacts: () -> ApplicationFacts,
) : NativePlugin {
    constructor(context: Context) : this({ readFacts(context) })

    override val name = "application"
    override val methods = intArrayOf(STATUS)

    private companion object {
        /** Reports the installed application's identity and version. */
        const val STATUS = 0
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (method != STATUS) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return ApplicationWire.snapshot(readFacts())
    }
}

/** The API level where package info takes flags rather than flags ints. */
private const val FLAGS_API = 33

/** The API level where the version code grew to 64 bits and signing info arrived. */
private const val SIGNING_API = 28

/** The API level that added install source details. */
private const val INSTALL_SOURCE_API = 30

/** Reads the installed package's record. */
internal fun readFacts(context: Context): ApplicationFacts {
    val manager = context.packageManager
    val info = packageInfo(context, 0)

    return ApplicationFacts(
        id = context.packageName,
        name = manager.getApplicationLabel(context.applicationInfo).toString(),
        version = info.versionName,
        build = versionCode(info).toString(),
        buildSignature = buildSignature(context),
        installerStore = installerStore(context),
        installTime = info.firstInstallTime.takeIf { it > 0 },
        updateTime = info.lastUpdateTime.takeIf { it > 0 },
    )
}

/** The installed package's info, read the way its API level allows. */
private fun packageInfo(
    context: Context,
    flags: Int,
): PackageInfo =
    try {
        if (Build.VERSION.SDK_INT >= FLAGS_API) {
            context.packageManager.getPackageInfo(context.packageName, PackageManager.PackageInfoFlags.of(flags.toLong()))
        } else {
            @Suppress("DEPRECATION")
            context.packageManager.getPackageInfo(context.packageName, flags)
        }
    } catch (error: PackageManager.NameNotFoundException) {
        throw PluginException(PluginException.NATIVE, error)
    }

/** The version code, which grew to a long in API 28. */
private fun versionCode(info: PackageInfo): Long =
    if (Build.VERSION.SDK_INT >= SIGNING_API) {
        info.longVersionCode
    } else {
        @Suppress("DEPRECATION")
        info.versionCode.toLong()
    }

/**
 * The SHA-256 digest of the certificate that signed this APK. After a key
 * rotation `apkContentsSigners` holds the current certificate, not the
 * original one at the start of the rotation history.
 */
@Suppress("PackageManagerGetSignatures")
internal fun buildSignature(context: Context): String? {
    val certificate =
        if (Build.VERSION.SDK_INT >= SIGNING_API) {
            packageInfo(context, PackageManager.GET_SIGNING_CERTIFICATES)
                .signingInfo
                ?.apkContentsSigners
                ?.firstOrNull()
        } else {
            @Suppress("DEPRECATION")
            packageInfo(context, PackageManager.GET_SIGNATURES).signatures?.firstOrNull()
        }

    return certificate?.let { ApplicationWire.sha256(it.toByteArray()) }
}

/**
 * The package that initiated this app's install, such as Google Play's
 * `com.android.vending`; `null` for `adb` and system installs.
 */
internal fun installerStore(context: Context): String? {
    val manager = context.packageManager

    return try {
        if (Build.VERSION.SDK_INT >= INSTALL_SOURCE_API) {
            manager.getInstallSourceInfo(context.packageName).initiatingPackageName
        } else {
            @Suppress("DEPRECATION")
            manager.getInstallerPackageName(context.packageName)
        }
    } catch (error: PackageManager.NameNotFoundException) {
        null
    }?.takeIf {
        it.isNotBlank() && it.toByteArray(Charsets.UTF_8).size <= ApplicationWire.MAX_INSTALLER_BYTES
    }
}
