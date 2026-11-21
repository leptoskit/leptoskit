# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

- **Android device-policy plugin.** `leptoskit_plugin_device_policy` exposes an explicit, least-privilege Device Admin / Device Policy Manager surface for Android: status without prompting, user-driven activation, lock, password quality and minimum length, camera disable, confirmed wipe, and admin removal. Non-Android platforms return `Unavailable`. Apps own the manifest receiver and the `device_admin.xml` policy file so they declare only the policies they need. Wipe requires `WipeRequest::factory_reset()`; empty payloads cannot factory-reset a device. Google Play and enterprise-policy constraints still apply and are documented with the API. _Owner:_ Ifiok Jr. · _Introduced in:_ [23482c7](https://github.com/leptoskit/leptoskit/commit/23482c7a0485da518e8aedd022a54d9c8a49338d)

### Fixes

- **Harden native-wave plugin correctness.** Live Activities on iOS cancel push-token waits without freezing the per-id busy lock, replace duplicate ActivityKit ids on start, and reject duplicate content keys. Picture-in-picture leases best-effort exit system PiP and observe user exit; `request_overlay` docs match launch-time status. Home widgets stop reporting unrelated pins for unknown kinds, commit preferences before success, and reject dangerous URL schemes. Device-policy and Live Activities stubs validate payloads before Unavailable. Wear/watchOS hosts return errors instead of panicking on viewport kind mismatch. New Android hosts declare `supportsPictureInPicture` on MainActivity. _Owner:_ Ifiok Jr. · _Introduced in:_ [879db6b](https://github.com/leptoskit/leptoskit/commit/879db6b2334d0c5bd8519f9a26f7a3eb92fca7f0)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-07)

### Features

#### leptoskit_plugin_device_policy: Android device admin

_Owner:_ [@ifiokjr](https://github.com/ifiokjr)

`leptoskit_plugin_device_policy` is the leptoskit plugin for explicit Android Device Admin / Device Policy Manager capabilities such as lock, password rules, camera disable, and confirmed wipe.

Add `leptoskit_plugin_device_policy` as a direct dependency and select the `device-policy` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::DevicePolicy` rather than resolving the plugin on every call. Only Android implements Device Admin / Device Policy Manager capabilities; every other platform returns `Unavailable`. Status never prompts. Privileged actions require an active device admin the user enabled, and wipe requires an explicit confirmation token.
