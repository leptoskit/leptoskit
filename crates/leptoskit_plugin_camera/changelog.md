# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-08)

### Features

#### leptoskit_plugin_camera: camera sessions with a Camera surface

_Owner:_ [@ifiokjr](https://github.com/ifiokjr)

`leptoskit_plugin_camera` is the leptoskit plugin for host-owned camera sessions. Create a session for the back or front lens, set torch when available, take a bounded still into app cache, snapshot status, and release the session. Show the session with a `Camera(session_id)` view that binds a host-owned id the way `Scene3d` and `Video` bind theirs.

Add `leptoskit_plugin_camera` as a direct dependency and select the `camera` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Guests enable the `guest` feature and reuse `guest::Camera`. Ask for camera access through the `permissions` plugin. iOS and macOS use `AVCaptureSession` with an `AVCaptureVideoPreviewLayer` surface; Android uses CameraX with a `PreviewView`; the browser uses `getUserMedia` into an HTML `<video>` element. Windows, Linux, and HarmonyOS return `Unavailable`. Video recording is not in this MVP.
