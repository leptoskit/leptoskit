<h1 align="center">leptoskit_plugin_device_keys</h1>

<p align="center">
  <strong>leptoskit_plugin_device_keys</strong> is the leptoskit plugin for hardware-bound signing keys gated by biometrics or the device passcode: P-256 keys that can never be copied off the device, signed through Apple <code>Secure Enclave</code> + <code>LAContext</code> and Android <code>AndroidKeyStore</code> + <code>BiometricPrompt</code>.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#what-is-guaranteed"><strong>What is guaranteed</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#verifying-on-a-server"><strong>Verifying on a server</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_device_keys"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_device_keys"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_device_keys.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_device_keys"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_device_keys/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

`device-keys` lets an app prove "the device owner just authenticated on this device" to a server with a key that can never leave the device. A screen that approves a sensitive action — a transfer, an account change, a signing operation — generates one key under an alias, then signs each approval through the operating system's biometric or passcode prompt. The server keeps the public key and verifies each signature against it.

| Platform        | Implementation                                      | Scope                                                                      |
| --------------- | --------------------------------------------------- | -------------------------------------------------------------------------- |
| iOS / macOS     | Swift `SecKeyCreateRandomKey` in the Secure Enclave | P-256 key, `LAContext` prompt, `.biometryCurrentSet` or `.userPresence`    |
| Android         | Kotlin `KeyPairGenerator` in `AndroidKeyStore`      | P-256 key, `BiometricPrompt` + `CryptoObject`, StrongBox with TEE fallback |
| Windows / Linux | Unavailable                                         | No native handler                                                          |
| Browser         | Unavailable                                         | WebAuthn is a different relying-party protocol and is not substituted      |

The guest API is `guest::DeviceKeys::{generate, public_key, sign, delete, available}`, behind the `guest` feature. `generate` returns the public key as an uncompressed SEC1 point (65 bytes) and as SPKI DER. `sign` returns the ECDSA P-256 / SHA-256 signature as DER and as the raw 64-byte `r || s` form.

## What is guaranteed

- **Non-exportable.** The private key is created inside the Secure Enclave or Android Keystore and never leaves it. No API returns the private key, and there is no software-key fallback: a platform without the hardware reports `Unavailable` instead of silently generating an exportable key.
- **Per-use authentication.** Every `sign` call shows the system prompt and re-authenticates the user. Apple keys use `.privateKeyUsage` with `.biometryCurrentSet` or `.userPresence`; Android keys use `setUserAuthenticationParameters(0, …)`, which requires authentication every use rather than caching it for the unlock interval.
- **Invalidation on biometric change.** A `Biometry` key is bound to the current enrolled biometric set. Apple uses `.biometryCurrentSet`; Android uses `setInvalidatedByBiometricEnrollment(true)`. When a fingerprint or face is added or removed, the key becomes unusable and `sign` returns `Invalidated` (or the underlying platform failure), after which the app should delete it and generate a new one.
- **No silent software fallback.** Desktop Rust hosts without a Swift/Kotlin handler, simulators, and browsers report `Unavailable`.

What is _not_ guaranteed: the plugin does not prove which specific person authenticated (the operating system's biometric is a device-local unlock, not an identity), it does not attest the device or the hardware key, and it does not prevent a rooted device from tampering with its own keystore. Treat a signature as evidence the device owner authenticated on that device, not as a substitute for account authentication.

## How it fits

<!-- {=p_device_keysSummary} -->

Add `leptoskit_plugin_device_keys` as a direct dependency and select the `device-keys` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Apple generates a P-256 key in the Secure Enclave and signs through `LAContext`; Android generates in `AndroidKeyStore` and signs through `BiometricPrompt`. Every signature is fresh proof that the device owner authenticated on this device just now; the key can never be copied off the device. Desktop hosts without a native handler, and browsers, report `Unavailable`.

<!-- {/p_device_keysSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_device_keys::{guest::DeviceKeys, Policy};

let keys = DeviceKeys::new()?;
// Create a key that requires a current fingerprint or face on every use.
let public = keys.generate("approvals.v1", Policy::Biometry).await?;
// Show the prompt and sign an approval payload.
let signature = keys.sign("approvals.v1", payload, "Approve this transfer").await?;
```

Apple keys are scoped to the app by the application tag (`<bundle id>.device-keys.<alias>`). An iOS app that uses Face ID must add `NSFaceIDUsageDescription` to `hosts/ios/Info.plist`, exactly as for the `local-authentication` plugin. Android requests `USE_BIOMETRIC` in the generated manifest; Android API 30+ is required for strong-biometric and device-credential policies, and earlier releases report `Unavailable`.

Aliases are 1–64 ASCII characters from `[A-Za-z0-9._-]`. Messages are 1–4096 bytes. Reasons are 1–200 characters, shown as the prompt title. `sign` returns `KeyError::NotFound`, `Cancelled`, `LockedOut`, `NotEnrolled`, `Invalidated`, `Unavailable`, or `Failed`; `generate` additionally returns `AlreadyExists`; `public_key` returns `None` for an absent alias.

## Verifying on a server

The server keeps the public key from `generate` and verifies each signature with the `p256` crate. Feed it the SPKI DER and the message:

```rust,ignore
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};

fn approve(public_spki: &[u8], message: &[u8], der: &[u8]) -> bool {
    let Ok(public) = VerifyingKey::from_sec1_der(public_spki) else { return false; };
    let Ok(signature) = Signature::from_der(der) else { return false; };
    public.verify(message, &signature).is_ok()
}
```

The same crate can verify the raw `r || s` form with `Signature::from_bytes`. Store and bind the public key to the account at enrollment time; a signature only proves that this device's owner authenticated on this device, so it should gate a single sensitive action, not replace login.

## Binary protocol

All integers are little-endian `u32`. Strings and byte blobs are `u32` byte lengths followed by strict UTF-8 or raw bytes. Every decoder rejects trailing bytes, truncated fields, invalid UTF-8, and out-of-domain values before calling a platform API.

| ID | Method     | Request                         | Response                                                       |
| -- | ---------- | ------------------------------- | -------------------------------------------------------------- |
| 0  | Generate   | Version, policy `u32`, alias    | Version, status, SEC1 point, SPKI DER                          |
| 1  | Public key | Alias                           | Version, status, presence `u32`, then SEC1 + SPKI when present |
| 2  | Sign       | Version, alias, message, reason | Version, status, DER signature                                 |
| 3  | Delete     | Alias                           | Version, status                                                |
| 4  | Available  | Empty                           | Version, hardware `u32`, biometry `u32`, enrolled `u32`        |

Hardware is `SecureEnclave=0`, `StrongBox=1`, `Tee=2`, or `None=3`. Biometry is `TouchId=0`, `FaceId=1`, `OpticId=2`, `Fingerprint=3`, `Face=4`, `Iris=5`, or `None=6`. Status `0` is success; `1`–`8` are `NotFound`, `AlreadyExists`, `Cancelled`, `LockedOut`, `NotEnrolled`, `Invalidated`, `Unavailable`, and `Failed`.

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_device_keys)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
