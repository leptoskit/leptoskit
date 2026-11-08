---
"release": minor
---

# Hardware-bound signing keys gated by biometrics

`leptoskit_plugin_device_keys` is a new runtime plugin, `device-keys`, for hardware-bound P-256 signing keys that can never be copied off the device. `generate` creates a non-exportable key under an alias with a `Biometry` or `BiometryOrPasscode` policy; `sign` shows the operating system's biometric or passcode prompt and returns an ECDSA P-256 / SHA-256 signature, so a server can treat it as fresh proof that the device owner authenticated on this device just now. `public_key`, `delete`, and `available` round out the API.

Apple generates the key with `SecKeyCreateRandomKey` in the Secure Enclave (`kSecAttrTokenIDSecureEnclave`, `kSecAttrKeyTypeECSECPrimeRandom` 256-bit) with `SecAccessControlCreateWithFlags` using `.privateKeyUsage` plus `.biometryCurrentSet` or `.userPresence`, and signs with `SecKeyCreateSignature(.ecdsaSignatureMessageX962SHA256)` through an `LAContext`. Android generates in `AndroidKeyStore` with `setUserAuthenticationRequired(true)` and `setUserAuthenticationParameters(0, AUTH_BIOMETRIC_STRONG [| AUTH_DEVICE_CREDENTIAL])` for per-use authentication, `setInvalidatedByBiometricEnrollment(true)` for the `Biometry` policy, and `setIsStrongBoxBacked(true)` with a TEE fallback, then signs through a `BiometricPrompt` `CryptoObject`. A platform without the hardware, including the Secure Enclave-less simulator and browsers, reports `Unavailable`; there is no silent software-key fallback.

`sign` returns `KeyError::{NotFound, Cancelled, LockedOut, NotEnrolled, Invalidated, Unavailable, Failed}`, `generate` additionally `AlreadyExists`, and the public key is returned as both an uncompressed SEC1 point (65 bytes) and SPKI DER so a server can verify with the `p256` crate.
