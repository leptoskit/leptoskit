# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

#### Camera sessions and Camera surface

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #688](https://github.com/leptoskit/leptoskit/pull/688)

`leptoskit_plugin_camera` is a new runtime plugin, `camera`, for host-owned camera sessions. Create or release a session for the back or front lens, set torch when the hardware supports it (otherwise `Unavailable`), take a bounded still into app cache (URI + width/height/mime/size matching `image_picker` Asset framing), and snapshot status. Show the same session with a new `Camera(session_id)` native widget kind that binds a host-owned id the way `Scene3d` and `Video` bind theirs (requires an accessible label or `aria_hidden`).

Ask for camera access through the `permissions` plugin; this plugin returns `PermissionDenied` when capture lacks a grant. iOS and macOS use `AVCaptureSession` / `AVCaptureVideoPreviewLayer`; Android uses CameraX / `PreviewView`; the browser uses `getUserMedia` into HTML `<video>`. Windows, Linux, and HarmonyOS return `Unavailable`. Video recording remains out of scope for this release.

### Fixes

- **Android plugins compile against the public SDK again.** Picture-in-picture listens through AndroidX, which is the type the platform stubs actually ship, and still notices a plain activity leaving PiP when it resumes. The camera plugin passes the failure itself to `PluginException`. Device keys read the SPKI bytes from `PublicKey` instead of `ECKey`, which has none. Calendar, contacts, print, SMS, mail, the web browser, and the media library set a string slice's byte order on the `ByteBuffer` `slice()` returns, because Android's `limit` returns `Buffer`. A calendar event reserves room for both text lengths as well as the timestamps, and decoding a contact advances the caller's buffer so the next record in a list can be read. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #738](https://github.com/leptoskit/leptoskit/pull/738)

#### Apple plugin and JNI builds on current toolchains

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #718](https://github.com/leptoskit/leptoskit/pull/718)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-08)

### Features

#### leptoskit_plugin_camera: camera sessions with a Camera surface

_Owner:_ [@ifiokjr](https://github.com/ifiokjr)

`leptoskit_plugin_camera` is the leptoskit plugin for host-owned camera sessions. Create a session for the back or front lens, set torch when available, take a bounded still into app cache, snapshot status, and release the session. Show the session with a `Camera(session_id)` view that binds a host-owned id the way `Scene3d` and `Video` bind theirs.

Add `leptoskit_plugin_camera` as a direct dependency and select the `camera` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Guests enable the `guest` feature and reuse `guest::Camera`. Ask for camera access through the `permissions` plugin. iOS and macOS use `AVCaptureSession` with an `AVCaptureVideoPreviewLayer` surface; Android uses CameraX with a `PreviewView`; the browser uses `getUserMedia` into an HTML `<video>` element. Windows, Linux, and HarmonyOS return `Unavailable`. Video recording is not in this MVP.
