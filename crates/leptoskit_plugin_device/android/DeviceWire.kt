package dev.leptoskit.plugins.device

import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Everything method 0 reports. `null` and zero mean unknown. */
internal data class DeviceFacts(
    val kind: Int = 0,
    val simulator: Boolean? = null,
    val totalMemory: Long = 0,
    val availableMemory: Long? = null,
    val totalDisk: Long = 0,
    val freeDisk: Long? = null,
    val cpuCount: Int = 0,
    val manufacturer: String? = null,
    val brand: String? = null,
    val model: String? = null,
    val osName: String? = null,
    val osVersion: String? = null,
    val architecture: String? = null,
    val kernelVersion: String? = null,
    val build: AndroidBuild = AndroidBuild(),
)

/** `Build` and `ActivityManager` details for the Android section. */
internal data class AndroidBuild(
    val sdkInt: Int = 0,
    val previewSdkInt: Int? = null,
    val lowRamDevice: Boolean? = null,
    val lowMemory: Boolean? = null,
    val buildTime: Long? = null,
    val baseOs: String? = null,
    val codename: String? = null,
    val incremental: String? = null,
    val release: String? = null,
    val securityPatch: String? = null,
    val board: String? = null,
    val bootloader: String? = null,
    val device: String? = null,
    val display: String? = null,
    val fingerprint: String? = null,
    val hardware: String? = null,
    val host: String? = null,
    val id: String? = null,
    val product: String? = null,
    val tags: String? = null,
    val type: String? = null,
    val supportedAbis: List<String> = emptyList(),
    val supported32BitAbis: List<String> = emptyList(),
    val supported64BitAbis: List<String> = emptyList(),
    val systemFeatures: List<String> = emptyList(),
) {
    /** The section's strings in wire order. */
    val strings: List<String?>
        get() =
            listOf(
                baseOs,
                codename,
                incremental,
                release,
                securityPatch,
                board,
                bootloader,
                device,
                display,
                fingerprint,
                hardware,
                host,
                id,
                product,
                tags,
                type,
            )
}

/** Everything method 1 reports. `null` where Android withholds it. */
internal data class DeviceIdentifiers(
    val deviceName: String? = null,
    val vendorId: String? = null,
    val machineId: String? = null,
)

/** The little-endian wire shared with Rust and Swift. */
internal object DeviceWire {
    const val SNAPSHOT_VERSION = 2
    const val IDENTIFIERS_VERSION = 1
    const val MAX_TEXT_BYTES = 4096
    const val MAX_IDENTIFIER_BYTES = 256
    const val MAX_SHORT_LIST = 16
    const val MAX_SHORT_ITEM_BYTES = 128
    const val MAX_FEATURES = 1024
    const val MAX_FEATURE_BYTES = 256

    /** The Android platform section's tag. */
    const val ANDROID_TAG = 2

    /** `Build.UNKNOWN`, spelled out so this file needs no Android classes. */
    private const val BUILD_UNKNOWN = "unknown"

    /** Treats blank values and `Build.UNKNOWN` as unknown. */
    fun known(value: String?): String? = value?.takeUnless { it.isBlank() || it == BUILD_UNKNOWN }

    /** Non-blank entries that fit a short list, in their original order. */
    fun shortList(items: List<String?>): List<String> =
        items
            .mapNotNull(::known)
            .filter { it.toByteArray(Charsets.UTF_8).size <= MAX_SHORT_ITEM_BYTES }
            .take(MAX_SHORT_LIST)

    /** Distinct feature names that fit the wire, sorted, at most [MAX_FEATURES]. */
    fun features(names: List<String>): List<String> =
        names
            .filter { it.isNotBlank() && it.toByteArray(Charsets.UTF_8).size <= MAX_FEATURE_BYTES }
            .distinct()
            .sorted()
            .take(MAX_FEATURES)

    /** A name the wire can carry, or `null` rather than a failed call. */
    fun boundedIdentifier(value: String?): String? =
        value?.takeIf { it.isNotBlank() && it.toByteArray(Charsets.UTF_8).size <= MAX_IDENTIFIER_BYTES }

    fun snapshot(facts: DeviceFacts): ByteArray {
        val output = WireWriter()
        output.u32(SNAPSHOT_VERSION)
        output.u32(facts.kind)
        output.tri(facts.simulator)
        output.u64(facts.totalMemory)
        output.optionalU64(facts.availableMemory)
        output.u64(facts.totalDisk)
        output.optionalU64(facts.freeDisk)
        output.u32(facts.cpuCount)

        for (text in listOf(
            facts.manufacturer,
            facts.brand,
            facts.model,
            // Android has no machine model identifier like iOS's `iPhone15,2`.
            null,
            facts.osName,
            facts.osVersion,
            facts.architecture,
            facts.kernelVersion,
        )) {
            output.text(text, MAX_TEXT_BYTES)
        }

        val build = facts.build
        output.u32(ANDROID_TAG)
        output.u32(build.sdkInt)
        output.optionalU32(build.previewSdkInt)
        output.tri(build.lowRamDevice)
        output.tri(build.lowMemory)
        output.optionalU64(build.buildTime)

        for (text in build.strings) {
            output.text(text, MAX_TEXT_BYTES)
        }

        for (abis in listOf(build.supportedAbis, build.supported32BitAbis, build.supported64BitAbis)) {
            output.list(abis, MAX_SHORT_LIST, MAX_SHORT_ITEM_BYTES)
        }

        output.list(build.systemFeatures, MAX_FEATURES, MAX_FEATURE_BYTES)

        return output.toByteArray()
    }

    fun identifiers(identifiers: DeviceIdentifiers): ByteArray {
        val output = WireWriter()
        output.u32(IDENTIFIERS_VERSION)

        for (text in listOf(identifiers.deviceName, identifiers.vendorId, identifiers.machineId)) {
            output.text(text, MAX_IDENTIFIER_BYTES)
        }

        return output.toByteArray()
    }
}

/** Appends little-endian fields. */
internal class WireWriter {
    private val output = ByteArrayOutputStream(RESULT_CAPACITY)
    private val word = ByteBuffer.allocate(Long.SIZE_BYTES).order(ByteOrder.LITTLE_ENDIAN)

    private companion object {
        /** Room for a typical snapshot with its feature list. */
        const val RESULT_CAPACITY = 4096
    }

    fun u32(value: Int) {
        word.clear()
        output.write(word.putInt(value).array(), 0, Int.SIZE_BYTES)
    }

    fun u64(value: Long) {
        word.clear()
        output.write(word.putLong(value).array(), 0, Long.SIZE_BYTES)
    }

    /** Unknown, no, yes as 0, 1, 2. */
    fun tri(value: Boolean?) =
        u32(
            when (value) {
                null -> 0
                false -> 1
                true -> 2
            },
        )

    /** A presence word, then the value or zero. */
    fun optionalU32(value: Int?) {
        u32(if (value == null) 0 else 1)
        u32(value ?: 0)
    }

    /** A presence word, then the value or zero. */
    fun optionalU64(value: Long?) {
        u32(if (value == null) 0 else 1)
        u64(value ?: 0)
    }

    /** A length-prefixed UTF-8 string; `null` and empty are both unknown. */
    fun text(
        value: String?,
        max: Int,
    ) {
        val bytes = (value ?: "").toByteArray(Charsets.UTF_8)

        if (bytes.size > max) throw PluginException(PluginException.TOO_LARGE)
        u32(bytes.size)
        output.write(bytes)
    }

    /** A count, then non-empty bounded strings. */
    fun list(
        items: List<String>,
        maxItems: Int,
        maxBytes: Int,
    ) {
        if (items.size > maxItems) throw PluginException(PluginException.TOO_LARGE)
        u32(items.size)

        for (item in items) {
            if (item.isEmpty()) throw PluginException(PluginException.INVALID_ARGUMENT)
            text(item, maxBytes)
        }
    }

    fun toByteArray(): ByteArray = output.toByteArray()
}
