package dev.leptoskit.plugins.devicekeys

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.Signature
import java.security.interfaces.ECKey
import java.security.spec.ECGenParameterSpec
import java.util.concurrent.Executor
import kotlin.coroutines.resume

/**
 * Hardware-bound P-256 signing keys in AndroidKeyStore, gated by strong
 * biometrics or the device credential. The private key never leaves the
 * keystore, so a signature is fresh proof of a just-now device-owner
 * authentication.
 */
class DeviceKeysPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "device-keys"
    override val methods = intArrayOf(GENERATE, PUBLIC_KEY, SIGN, DELETE, AVAILABLE)
    private val handler = Handler(Looper.getMainLooper())
    private val executor = Executor { command -> handler.post(command) }
    private val application = context.applicationContext.packageName
    private var closed = false
    private var probedHardware: Int? = null

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Device-keys dispatch requires the UI thread" }

        if (closed) throw PluginException(PluginException.CANCELLED)

        return when (method) {
            GENERATE -> {
                generate(GenerateRequest.decode(payload))
            }

            PUBLIC_KEY -> {
                publicKey(decodeAlias(payload))
            }

            SIGN -> {
                sign(SignRequest.decode(payload))
            }

            DELETE -> {
                delete(decodeAlias(payload))
            }

            AVAILABLE -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                available()
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }
    }

    override fun close() {
        closed = true
    }

    /** Reports hardware, biometric hardware, and enrollment without prompting. */
    private fun available(): ByteArray {
        val enrolled = enrolled()

        return deviceKeysFields(WIRE_VERSION, probeHardware(), biometryKind(), if (enrolled) 1 else 0)
    }

    /** Creates a permanent, non-exportable P-256 key in AndroidKeyStore. */
    private fun generate(request: GenerateRequest): ByteArray {
        if (Build.VERSION.SDK_INT < KEYSTORE_USER_AUTH_API) {
            return deviceKeysFields(WIRE_VERSION, STATUS_UNAVAILABLE)
        }

        val alias = aliasFor(request.alias)
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

        if (keyStore.containsAlias(alias)) return deviceKeysFields(WIRE_VERSION, STATUS_ALREADY_EXISTS)

        val authenticators = authenticators(request.policy)

        return try {
            try {
                generateKey(alias, authenticators, request.policy, strongBox = true)
            } catch (_: StrongBoxUnavailableException) {
                generateKey(alias, authenticators, request.policy, strongBox = false)
            }
            publicKeyReply(alias)
        } catch (_: Exception) {
            deviceKeysFields(WIRE_VERSION, STATUS_UNAVAILABLE)
        }
    }

    /** Initializes and runs one key-pair generation. */
    private fun generateKey(
        alias: String,
        authenticators: Int,
        policy: Int,
        strongBox: Boolean,
    ) {
        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
        val builder =
            KeyGenParameterSpec
                .Builder(alias, KeyProperties.PURPOSE_SIGN)
                .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
                .setDigests(KeyProperties.DIGEST_SHA256)
                .setUserAuthenticationRequired(true)
                .setUserAuthenticationParameters(0, authenticators)

        if (policy == POLICY_BIOMETRY) builder.setInvalidatedByBiometricEnrollment(true)

        if (strongBox && Build.VERSION.SDK_INT >= STRONGBOX_API) builder.setIsStrongBoxBacked(true)

        generator.initialize(builder.build())
        generator.generateKeyPair()
    }

    /** Reads a key's public key, or reports absence. */
    private fun publicKey(alias: String): ByteArray {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

        if (!keyStore.containsAlias(aliasFor(alias))) {
            return deviceKeysFields(WIRE_VERSION, STATUS_OK, 0)
        }

        return publicKeyReply(aliasFor(alias))
    }

    /** Shows the system prompt with `reason`, then signs the message. */
    private suspend fun sign(request: SignRequest): ByteArray {
        val alias = aliasFor(request.alias)
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val entry =
            keyStore.getEntry(alias, null) as? KeyStore.PrivateKeyEntry
                ?: return deviceKeysFields(WIRE_VERSION, STATUS_NOT_FOUND)

        val signature =
            try {
                Signature.getInstance("SHA256withECDSA").apply { initSign(entry.privateKey) }
            } catch (_: KeyPermanentlyInvalidatedException) {
                return deviceKeysFields(WIRE_VERSION, STATUS_INVALIDATED)
            } catch (_: Exception) {
                return deviceKeysFields(WIRE_VERSION, STATUS_FAILED)
            }

        val activity = promptableActivity()

        return suspendCancellableCoroutine { continuation ->
            val signal = CancellationSignal()
            continuation.invokeOnCancellation { signal.cancel() }

            val prompt =
                BiometricPrompt
                    .Builder(activity)
                    .setTitle(request.reason)
                    .setAllowedAuthenticators(promptAuthenticators())
                    .setNegativeButton("Cancel", executor) { _, _ ->
                        finish(continuation, deviceKeysFields(WIRE_VERSION, STATUS_CANCELLED))
                    }.build()

            prompt.authenticate(
                BiometricPrompt.CryptoObject(signature),
                signal,
                executor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        try {
                            val crypto =
                                result.cryptoObject?.signature
                                    ?: throw IllegalStateException("Missing crypto signature")
                            crypto.update(request.message)
                            val der = crypto.sign()
                            finish(continuation, deviceKeysFields(WIRE_VERSION, STATUS_OK) + blob(der))
                        } catch (_: KeyPermanentlyInvalidatedException) {
                            finish(continuation, deviceKeysFields(WIRE_VERSION, STATUS_INVALIDATED))
                        } catch (_: Exception) {
                            finish(continuation, deviceKeysFields(WIRE_VERSION, STATUS_FAILED))
                        }
                    }

                    override fun onAuthenticationError(
                        errorCode: Int,
                        errString: CharSequence,
                    ) {
                        finish(continuation, deviceKeysFields(WIRE_VERSION, statusOf(errorCode)))
                    }

                    override fun onAuthenticationFailed() {}
                },
            )
        }
    }

    /** Deletes a key. Deleting an absent alias succeeds. */
    private fun delete(alias: String): ByteArray {
        KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.deleteEntry(aliasFor(alias))

        return deviceKeysFields(WIRE_VERSION, STATUS_OK)
    }

    /** The activity a system prompt can attach to, or an error saying why not. */
    private fun promptableActivity(): Activity {
        if (Build.VERSION.SDK_INT < KEYSTORE_USER_AUTH_API) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        if (context.checkSelfPermission(Manifest.permission.USE_BIOMETRIC) != PackageManager.PERMISSION_GRANTED) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }

        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)

        if (activity.isFinishing || activity.isDestroyed || activity.window?.decorView?.isShown != true) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        return activity
    }

    /** Encodes a stored public key as its SEC1 point and SPKI DER. */
    private fun publicKeyReply(alias: String): ByteArray {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val entry =
            keyStore.getEntry(alias, null) as? KeyStore.PrivateKeyEntry
                ?: return deviceKeysFields(WIRE_VERSION, STATUS_FAILED)
        val publicKey =
            entry.certificate.publicKey as? ECKey
                ?: return deviceKeysFields(WIRE_VERSION, STATUS_FAILED)
        val spki = publicKey.encoded
        // The SPKI BIT STRING ends with the uncompressed 65-byte point.
        val sec1 = spki.copyOfRange(spki.size - SEC1_BYTES, spki.size)

        return deviceKeysFields(WIRE_VERSION, STATUS_OK, 1) + blob(sec1) + blob(spki)
    }

    /** The authenticators a generated key accepts. */
    private fun authenticators(policy: Int): Int =
        if (policy == POLICY_BIOMETRY_OR_PASSCODE) {
            BiometricManager.Authenticators.BIOMETRIC_STRONG or
                BiometricManager.Authenticators.DEVICE_CREDENTIAL
        } else {
            BiometricManager.Authenticators.BIOMETRIC_STRONG
        }

    /** Prompt authenticators: strong biometrics or the device credential. */
    private fun promptAuthenticators(): Int =
        BiometricManager.Authenticators.BIOMETRIC_STRONG or
            BiometricManager.Authenticators.DEVICE_CREDENTIAL

    /** The biometric hardware kind the device reports. */
    private fun biometryKind(): Int {
        val manager = context.packageManager

        return when {
            manager.hasSystemFeature(PackageManager.FEATURE_FINGERPRINT) -> BIOMETRY_FINGERPRINT
            manager.hasSystemFeature(PackageManager.FEATURE_FACE) -> BIOMETRY_FACE
            manager.hasSystemFeature(PackageManager.FEATURE_IRIS) -> BIOMETRY_IRIS
            else -> BIOMETRY_NONE
        }
    }

    /** Whether a strong biometric is enrolled. */
    private fun enrolled(): Boolean {
        if (Build.VERSION.SDK_INT < KEYSTORE_USER_AUTH_API) return false

        val manager = context.getSystemService(BiometricManager::class.java) ?: return false

        return manager.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG) ==
            BiometricManager.BIOMETRIC_SUCCESS
    }

    /** Probes StrongBox, then TEE, then reports none, caching the result. */
    private fun probeHardware(): Int {
        if (Build.VERSION.SDK_INT < KEYSTORE_API) return HARDWARE_NONE

        probedHardware?.let { return it }

        probedHardware =
            try {
                generateProbe(strongBox = true)
                HARDWARE_STRONGBOX
            } catch (_: Exception) {
                try {
                    generateProbe(strongBox = false)
                    HARDWARE_TEE
                } catch (_: Exception) {
                    HARDWARE_NONE
                }
            }

        return probedHardware!!
    }

    /** Generates and immediately deletes a probe key to learn the keystore kind. */
    private fun generateProbe(strongBox: Boolean) {
        val alias = "$application.device-keys.probe"
        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
        val builder =
            KeyGenParameterSpec
                .Builder(alias, KeyProperties.PURPOSE_SIGN)
                .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
                .setDigests(KeyProperties.DIGEST_SHA256)

        if (strongBox && Build.VERSION.SDK_INT >= STRONGBOX_API) builder.setIsStrongBoxBacked(true)

        generator.initialize(builder.build())
        generator.generateKeyPair()
        KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.deleteEntry(alias)
    }

    /** The application-scoped alias that names one key. */
    private fun aliasFor(alias: String): String = "$application.device-keys.$alias"

    /** Resumes the continuation when it is still active. */
    private fun finish(
        continuation: CancellableContinuation<ByteArray>,
        value: ByteArray,
    ) {
        if (continuation.isActive) continuation.resume(value)
    }

    /** The terminal status for one BiometricPrompt error code. */
    private fun statusOf(errorCode: Int): Int =
        when (errorCode) {
            BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED,
            BiometricPrompt.BIOMETRIC_ERROR_CANCELED,
            -> STATUS_CANCELLED

            BiometricPrompt.BIOMETRIC_ERROR_LOCKOUT,
            BiometricPrompt.BIOMETRIC_ERROR_LOCKOUT_PERMANENT,
            -> STATUS_LOCKED_OUT

            BiometricPrompt.BIOMETRIC_ERROR_NO_BIOMETRICS,
            BiometricPrompt.BIOMETRIC_ERROR_NO_DEVICE_CREDENTIAL,
            -> STATUS_NOT_ENROLLED

            BiometricPrompt.BIOMETRIC_ERROR_HW_NOT_PRESENT,
            BiometricPrompt.BIOMETRIC_ERROR_HW_UNAVAILABLE,
            -> STATUS_UNAVAILABLE

            else -> STATUS_FAILED
        }

    /** Length-prefixes one binary blob. */
    private fun blob(value: ByteArray): ByteArray =
        ByteBuffer
            .allocate(4 + value.size)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(value.size)
            .put(value)
            .array()

    private companion object {
        /** Methods the Rust crate calls. */
        const val GENERATE = 0
        const val PUBLIC_KEY = 1
        const val SIGN = 2
        const val DELETE = 3
        const val AVAILABLE = 4

        /** The first API level with AndroidKeyStore at all. */
        const val KEYSTORE_API = 23

        /** The first API level with StrongBox-backed keys. */
        const val STRONGBOX_API = 28

        /** The first API level with per-use strong-biometric auth. */
        const val KEYSTORE_USER_AUTH_API = 30

        /** An uncompressed SEC1 point is 65 bytes. */
        const val SEC1_BYTES = 65
    }
}
