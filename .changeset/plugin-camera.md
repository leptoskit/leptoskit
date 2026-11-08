---
"leptoskit_plugin_camera": minor
"leptoskit_native": minor
---

# Camera sessions and Camera surface

`leptoskit_plugin_camera` is a new runtime plugin, `camera`, for host-owned camera sessions. Create or release a session for the back or front lens, set torch when the hardware supports it (otherwise `Unavailable`), take a bounded still into app cache (URI + width/height/mime/size matching `image_picker` Asset framing), and snapshot status. Show the same session with a new `Camera(session_id)` native widget kind that binds a host-owned id the way `Scene3d` and `Video` bind theirs (requires an accessible label or `aria_hidden`).

Ask for camera access through the `permissions` plugin; this plugin returns `PermissionDenied` when capture lacks a grant. iOS and macOS use `AVCaptureSession` / `AVCaptureVideoPreviewLayer`; Android uses CameraX / `PreviewView`; the browser uses `getUserMedia` into HTML `<video>`. Windows, Linux, and HarmonyOS return `Unavailable`. Video recording remains out of scope for this release.
