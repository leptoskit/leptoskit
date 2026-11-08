package dev.leptoskit.plugins.cellular

import android.content.Context
import android.os.Build
import android.telephony.TelephonyManager
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Permission-light cellular snapshot. Declares no READ_PHONE_STATE / location
 * permissions: SecurityException becomes Restricted, empty readings stay
 * Unknown, and hosts without telephony stay Unavailable.
 */
class CellularPlugin(
    context: Context,
) : NativePlugin {
    override val name = "cellular"
    override val methods = intArrayOf(SNAPSHOT)

    private val appContext = context.applicationContext

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (method != SNAPSHOT) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return encode(snapshot())
    }

    private fun snapshot(): Snapshot {
        val manager =
            appContext.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager
                ?: return Snapshot.unavailable()

        if (manager.phoneType == TelephonyManager.PHONE_TYPE_NONE) {
            return Snapshot.unavailable()
        }

        return try {
            Snapshot(
                carrierName = textField(manager.networkOperatorName),
                mcc = codeField(operatorPart(manager.networkOperator, 0, 3)),
                mnc = codeField(operatorPart(manager.networkOperator, 3, null)),
                generation = generation(manager),
                allowsVoip = UNKNOWN,
            )
        } catch (_: SecurityException) {
            Snapshot.restricted()
        }
    }

    private fun textField(value: String?): TextField {
        val trimmed = value?.trim().orEmpty()

        return if (trimmed.isEmpty()) TextField.Unknown else TextField.Ready(trimmed)
    }

    private fun codeField(value: String?): CodeField {
        val trimmed = value?.trim().orEmpty()

        if (trimmed.isEmpty()) return CodeField.Unknown

        val code = trimmed.toIntOrNull() ?: return CodeField.Unknown

        if (code !in 0..65535) return CodeField.Unknown

        return CodeField.Ready(code)
    }

    private fun operatorPart(
        operator: String?,
        start: Int,
        end: Int?,
    ): String? {
        val trimmed = operator?.trim().orEmpty()

        if (trimmed.length < start) return null

        return if (end == null) {
            if (trimmed.length <= start) null else trimmed.substring(start)
        } else {
            if (trimmed.length < end) null else trimmed.substring(start, end)
        }
    }

    private fun generation(manager: TelephonyManager): Int =
        try {
            val networkType =
                if (Build.VERSION.SDK_INT >= 24) {
                    manager.dataNetworkType
                } else {
                    @Suppress("DEPRECATION")
                    manager.networkType
                }

            when (networkType) {
                TelephonyManager.NETWORK_TYPE_GPRS,
                TelephonyManager.NETWORK_TYPE_EDGE,
                TelephonyManager.NETWORK_TYPE_CDMA,
                TelephonyManager.NETWORK_TYPE_1xRTT,
                TelephonyManager.NETWORK_TYPE_IDEN,
                -> TWO_G

                TelephonyManager.NETWORK_TYPE_UMTS,
                TelephonyManager.NETWORK_TYPE_EVDO_0,
                TelephonyManager.NETWORK_TYPE_EVDO_A,
                TelephonyManager.NETWORK_TYPE_HSDPA,
                TelephonyManager.NETWORK_TYPE_HSUPA,
                TelephonyManager.NETWORK_TYPE_HSPA,
                TelephonyManager.NETWORK_TYPE_EVDO_B,
                TelephonyManager.NETWORK_TYPE_EHRPD,
                TelephonyManager.NETWORK_TYPE_HSPAP,
                -> THREE_G

                TelephonyManager.NETWORK_TYPE_LTE,
                -> FOUR_G

                TelephonyManager.NETWORK_TYPE_NR,
                -> FIVE_G

                TelephonyManager.NETWORK_TYPE_UNKNOWN,
                -> UNKNOWN

                else -> UNKNOWN
            }
        } catch (_: SecurityException) {
            RESTRICTED
        }

    private fun encode(snapshot: Snapshot): ByteArray {
        val result = ByteArrayOutputStream(64)
        write(result, snapshot.generation)
        write(result, snapshot.allowsVoip)
        write(result, snapshot.mcc.status)
        write(result, snapshot.mcc.value)
        write(result, snapshot.mnc.status)
        write(result, snapshot.mnc.value)
        write(result, snapshot.carrierName.status)
        writeText(result, snapshot.carrierName.text)

        return result.toByteArray()
    }

    private fun write(
        result: ByteArrayOutputStream,
        value: Int,
    ) {
        result.write(
            ByteBuffer
                .allocate(4)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(value)
                .array(),
        )
    }

    private fun writeText(
        result: ByteArrayOutputStream,
        text: String,
    ) {
        val bytes = text.toByteArray(Charsets.UTF_8)

        if (bytes.size > MAX_CARRIER_BYTES) throw PluginException(PluginException.TOO_LARGE)
        write(result, bytes.size)
        result.write(bytes)
    }

    private data class Snapshot(
        val carrierName: TextField,
        val mcc: CodeField,
        val mnc: CodeField,
        val generation: Int,
        val allowsVoip: Int,
    ) {
        companion object {
            fun unavailable() =
                Snapshot(
                    carrierName = TextField.Unavailable,
                    mcc = CodeField.Unavailable,
                    mnc = CodeField.Unavailable,
                    generation = UNAVAILABLE,
                    allowsVoip = UNAVAILABLE,
                )

            fun restricted() =
                Snapshot(
                    carrierName = TextField.Restricted,
                    mcc = CodeField.Restricted,
                    mnc = CodeField.Restricted,
                    generation = RESTRICTED,
                    allowsVoip = RESTRICTED,
                )
        }
    }

    private sealed class TextField {
        abstract val status: Int
        abstract val text: String

        data object Unknown : TextField() {
            override val status = UNKNOWN
            override val text = ""
        }

        data object Unavailable : TextField() {
            override val status = UNAVAILABLE
            override val text = ""
        }

        data object Restricted : TextField() {
            override val status = RESTRICTED
            override val text = ""
        }

        data class Ready(
            override val text: String,
        ) : TextField() {
            override val status = READY
        }
    }

    private sealed class CodeField {
        abstract val status: Int
        abstract val value: Int

        data object Unknown : CodeField() {
            override val status = UNKNOWN
            override val value = 0
        }

        data object Unavailable : CodeField() {
            override val status = UNAVAILABLE
            override val value = 0
        }

        data object Restricted : CodeField() {
            override val status = RESTRICTED
            override val value = 0
        }

        data class Ready(
            override val value: Int,
        ) : CodeField() {
            override val status = READY
        }
    }

    private companion object {
        const val SNAPSHOT = 0
        const val UNKNOWN = 0
        const val UNAVAILABLE = 1
        const val RESTRICTED = 2
        const val READY = 3
        const val TWO_G = 3
        const val THREE_G = 4
        const val FOUR_G = 5
        const val FIVE_G = 6
        const val MAX_CARRIER_BYTES = 256
    }
}
