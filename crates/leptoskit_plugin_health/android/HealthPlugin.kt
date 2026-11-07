package dev.leptoskit.plugins.health

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.health.connect.client.HealthConnectClient
import androidx.health.connect.client.HealthConnectFeatures
import androidx.health.connect.client.PermissionController
import androidx.health.connect.client.changes.DeletionChange
import androidx.health.connect.client.changes.UpsertionChange
import androidx.health.connect.client.permission.HealthPermission
import androidx.health.connect.client.records.ExerciseSessionRecord
import androidx.health.connect.client.records.HeartRateRecord
import androidx.health.connect.client.records.HeartRateVariabilityRmssdRecord
import androidx.health.connect.client.records.OxygenSaturationRecord
import androidx.health.connect.client.records.Record
import androidx.health.connect.client.records.RespiratoryRateRecord
import androidx.health.connect.client.records.RestingHeartRateRecord
import androidx.health.connect.client.records.SleepSessionRecord
import androidx.health.connect.client.records.StepsRecord
import androidx.health.connect.client.request.ChangesTokenRequest
import androidx.health.connect.client.request.ReadRecordsRequest
import androidx.health.connect.client.time.TimeRangeFilter
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.time.Instant
import kotlin.reflect.KClass

/**
 * Reads Health Connect records. The permission screen is Health Connect's own,
 * launched through the host's activity-result broker; reads never prompt.
 */
class HealthPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "health"
    override val methods = intArrayOf(AVAILABILITY, FEATURES, GRANTED, REQUEST, READ, CHANGES_TOKEN, CHANGES)
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        return when (method) {
            AVAILABILITY -> availability()
            FEATURES -> features(client())
            GRANTED -> grantedReply(client())
            REQUEST -> requestPermissions(TypesRequest(payload))
            READ -> read(ReadRequest(payload))
            CHANGES_TOKEN -> changesToken(TypesRequest(payload))
            else -> changes(payload)
        }
    }

    /** Whether this device has a usable Health Connect provider. */
    private fun availability(): ByteArray =
        when (HealthConnectClient.getSdkStatus(context)) {
            HealthConnectClient.SDK_AVAILABLE -> AVAILABLE
            HealthConnectClient.SDK_UNAVAILABLE_PROVIDER_UPDATE_REQUIRED -> NEEDS_UPDATE
            else -> if (Build.VERSION.SDK_INT >= MIN_PROVIDER_API) NOT_INSTALLED else UNSUPPORTED
        }.let { code -> byteArrayOf(code.toByte(), 0, 0, 0) }

    /** The types and extra permissions this provider supports. */
    private fun features(client: HealthConnectClient): ByteArray {
        val status = client.features
        val background = status.getFeatureStatus(HealthConnectFeatures.FEATURE_READ_HEALTH_DATA_IN_BACKGROUND)
        val history = status.getFeatureStatus(HealthConnectFeatures.FEATURE_READ_HEALTH_DATA_HISTORY)
        var flags = 0
        if (background == HealthConnectFeatures.FEATURE_STATUS_AVAILABLE) flags = flags or FLAG_BACKGROUND
        if (history == HealthConnectFeatures.FEATURE_STATUS_AVAILABLE) flags = flags or FLAG_HISTORY
        return encodeMask(ANDROID_TYPES, flags)
    }

    /** What the user has allowed, without prompting. */
    private suspend fun grantedReply(client: HealthConnectClient): ByteArray {
        val (mask, flags) = grantedState(client)
        return encodeMask(mask, flags)
    }

    /** Shows Health Connect's permission screen and reports what it left behind. */
    private suspend fun requestPermissions(request: TypesRequest): ByteArray {
        val client = client()
        val wanted = permissionsOf(request.mask, request.flags)
        if (request.flags != 0) requireDeclared(wanted)
        val host = context as? NativeActivityResultHost ?: throw PluginException(PluginException.UNAVAILABLE)

        val contract = PermissionController.createRequestPermissionResultContract()
        val intent = contract.createIntent(context, wanted)
        host.launchPluginActivity(intent)
        if (closed) throw PluginException(PluginException.CANCELLED)
        return grantedReply(client)
    }

    /** One page of one type's records, oldest first. */
    private suspend fun read(request: ReadRequest): ByteArray {
        val client = client()
        val start = Instant.ofEpochMilli(request.startMs)
        val end = Instant.ofEpochMilli(request.endMs)
        val response = client.readRecords(readRequest(request, TimeRangeFilter.between(start, end)))
        return encodePage(response.records, response.pageToken)
    }

    /** A token the provider moves forward as records change. */
    private suspend fun changesToken(request: TypesRequest): ByteArray {
        val client = client()
        val classes = RECORD_TYPES.filter { (type, _) -> request.mask and (1 shl type) != 0 }.values.toSet()
        val token = client.getChangesToken(ChangesTokenRequest(classes))
        if (token.isEmpty()) throw PluginException(PluginException.NATIVE)
        return WireWriter().string(token).bytes()
    }

    /** The records written and deleted after a token. */
    private suspend fun changes(payload: ByteBuffer): ByteArray {
        val reader = WireReader(payload)
        val token = reader.string()
        reader.finished()
        val response = client().getChanges(token)
        val writer = WireWriter()

        if (response.changesTokenExpired) {
            writer.u32(0)
            writer.u32(0)
            writer.optional(null)
            writer.u32(FLAG_EXPIRED)
            return writer.bytes()
        }

        val records = response.changes.mapNotNull { change -> (change as? UpsertionChange)?.record }
        val deleted = response.changes.mapNotNull { change -> (change as? DeletionChange)?.recordId }
        writer.u32(records.size)
        records.forEach { record -> writer.raw(encodeRecord(record)) }
        writer.u32(deleted.size)
        deleted.forEach { id -> writer.string(id) }
        writer.optional(response.nextChangesToken)
        writer.u32(if (response.hasMore) FLAG_HAS_MORE else 0)
        return writer.bytes()
    }

    /** The provider, when this device has one; errors say why not. */
    private fun client(): HealthConnectClient {
        if (HealthConnectClient.getSdkStatus(context) != HealthConnectClient.SDK_AVAILABLE) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        return HealthConnectClient.getOrCreate(context)
    }

    override fun close() {
        closed = true
    }

    /** The granted type mask plus flag bits, with `exact` always set. */
    private suspend fun grantedState(client: HealthConnectClient): Pair<Int, Int> {
        val granted = client.permissionController.getGrantedPermissions()
        val mask =
            RECORD_TYPES.entries.fold(0) { mask, (type, _) ->
                if (readPermission(type) in granted) mask or (1 shl type) else mask
            }
        var flags = FLAG_EXACT
        if (HealthPermission.PERMISSION_READ_HEALTH_DATA_IN_BACKGROUND in granted) {
            flags = flags or FLAG_BACKGROUND
        }
        if (HealthPermission.PERMISSION_READ_HEALTH_DATA_HISTORY in granted) flags = flags or FLAG_HISTORY
        return Pair(mask, flags)
    }

    /** The read permissions a type mask and extra flags ask for. */
    private fun permissionsOf(
        mask: Int,
        extra: Int,
    ): Set<String> {
        val wanted =
            RECORD_TYPES.keys
                .filter { type -> mask and (1 shl type) != 0 }
                .map(::readPermission)
                .toMutableSet()
        if (extra and FLAG_BACKGROUND != 0) wanted += HealthPermission.PERMISSION_READ_HEALTH_DATA_IN_BACKGROUND
        if (extra and FLAG_HISTORY != 0) wanted += HealthPermission.PERMISSION_READ_HEALTH_DATA_HISTORY
        return wanted
    }

    /** Extras are app-owned manifest permissions; asking without one is a caller error. */
    @Suppress("DEPRECATION")
    private fun requireDeclared(permissions: Set<String>) {
        val declared =
            context.packageManager
                .getPackageInfo(context.packageName, PackageManager.GET_PERMISSIONS)
                .requestedPermissions
                .orEmpty()

        if (permissions.any { it !in declared }) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    private companion object {
        const val AVAILABILITY = 0
        const val FEATURES = 1
        const val GRANTED = 2
        const val REQUEST = 3
        const val READ = 4
        const val CHANGES_TOKEN = 5
        const val CHANGES = 6

        /** Availability codes, in wire order. */
        const val AVAILABLE = 0
        const val NOT_INSTALLED = 1
        const val NEEDS_UPDATE = 2
        const val UNSUPPORTED = 3

        /** Reply flag bits. */
        const val FLAG_BACKGROUND = 1
        const val FLAG_HISTORY = 2
        const val FLAG_EXACT = 4
        const val FLAG_EXPIRED = 1
        const val FLAG_HAS_MORE = 2

        /** The oldest Android the Health Connect app installs on. */
        const val MIN_PROVIDER_API = 28
    }
}

/** The record class and read permission each wire type maps to. */
internal val RECORD_TYPES: Map<Int, KClass<out Record>> =
    mapOf(
        TYPE_HEART_RATE to HeartRateRecord::class,
        TYPE_RESTING_HEART_RATE to RestingHeartRateRecord::class,
        TYPE_HRV_RMSSD to HeartRateVariabilityRmssdRecord::class,
        TYPE_STEPS to StepsRecord::class,
        TYPE_SLEEP to SleepSessionRecord::class,
        TYPE_OXYGEN_SATURATION to OxygenSaturationRecord::class,
        TYPE_RESPIRATORY_RATE to RespiratoryRateRecord::class,
        TYPE_EXERCISE to ExerciseSessionRecord::class,
    )

/** The read permission one wire type asks for, from the record it reads. */
internal fun readPermission(type: Int): String = HealthPermission.getReadPermission(recordClassOf(type))

/** A page request over the record class the wire type names. */
@Suppress("UNCHECKED_CAST")
private fun readRequest(
    request: ReadRequest,
    filter: TimeRangeFilter,
): ReadRecordsRequest<Record> =
    ReadRecordsRequest(
        recordClassOf(request.type) as KClass<Record>,
        filter,
        emptySet(),
        true,
        request.pageSize,
        request.token,
    )

private fun recordClassOf(type: Int): KClass<out Record> = RECORD_TYPES[type] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
