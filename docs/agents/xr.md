# XR: augmented and virtual reality

`leptoskit_xr` gives an app the real world: phone AR, headset VR and passthrough AR, a rotation-sensor fallback, and recorded playback. The public pages are [augmented reality on phones](../src/guide/34-augmented-reality.md), [VR and AR on headsets](../src/guide/35-headsets.md), and [Meta Quest](../src/platforms/quest.md). This note records the design, the platform choices, and what has and hasn't been checked. The first consumer is Reapers, an AR game being ported from Flutter.

## Shape

- **Tracking and rendering stay on the host.** The Wasm guest runs in an interpreter; it can't touch the camera or the GPU and shouldn't copy camera frames through linear memory. The guest gets a `Frame` (pose, views, planes, anchors, light, inputs, whether depth exists) and places its objects; the host renders over the camera picture or into the headset, hiding the scene behind real depth.
- **One runtime, many backends.** `host::Backend` is the platform; `host::XrRuntime` owns one backend and at most one session. Something calls `XrRuntime::update(time)` once per display frame: the renderer's loop, or `XrRuntime::drive`'s thread when nothing renders (60 Hz while a session runs, 20 Hz idle).
- **A pull stream, bounded by construction.** `NEXT_FRAME { after, planes_revision }` long-polls for a frame newer than `after`. The runtime publishes at most `frame_rate` frames a second (the session asks; `[native.plugin-options.xr] max-frame-rate` caps it) and only ever the newest, so a slow guest skips frames instead of queueing them. Planes travel only when their revision changes (`PlaneCache` on the guest), and every list in a frame has a bound (64 planes of 64 points, 64 anchors, 8 inputs, 4 views); the largest reply is 256 KiB.
- **Plugin shape.** An async Rust plugin (`rust-async`, context factory `host::from_context`), so it needs `[native.plugin-options.xr]` (an empty table is enough). The plugin is `Unavailable` on macOS and HarmonyOS by metadata. `apple-frameworks` (new CLI metadata) links ARKit, AVFoundation, CoreMotion, CoreVideo, and Metal into the generated iOS app, which a Rust static library can't ask Xcode for itself.
- **Coordinates.** Right-handed metres, +Y up, viewers look down −Z, column-major matrices, projections with 0..1 depth. Platform projection matrices are converted to field-of-view angles (`Fov::from_matrix`), so the renderer chooses its own clip planes.
- **Determinism.** `ReplayBackend` reads no clock: the frame at `t` is the last recorded at or before `t − start`. Recordings are `LKXR` files (`Recording::to_bytes`), capped at ten minutes of 60 Hz frames and 4 MiB depth pictures.

## Backends

| Backend     | Where                                                 | Implemented                                                                                                                         | Checked                                |
| ----------- | ----------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| Replay      | everywhere native                                     | Playback, looping, recording, software hit tests and anchors                                                                        | Unit and integration tests             |
| Orientation | iOS (Core Motion), Android (NDK game rotation vector) | 3DoF with recentred heading and screen rotation                                                                                     | Math tested; sources compile-only      |
| ARKit       | iOS arm64                                             | World tracking, planes with labels and outlines, smoothed LiDAR depth and confidence, light, raycasts, anchors                      | Compiles for `aarch64-apple-ios(-sim)` |
| ARCore      | Android                                               | Planes, Depth API, ambient light, native hit tests, anchors, install and permission checks; `libarcore_sdk_c.so` opened at run time | Compiles for `aarch64-linux-android`   |
| OpenXR      | Android (Quest, Android XR), Windows, Linux           | Instance, stereo views, Touch and simple controllers, `XR_EXT_hand_tracking`, `XR_FB_passthrough`, floor spaces                     | Compiles for `aarch64-linux-android`   |
| WebXR       | browser (`web` feature)                               | `immersive-ar`/`-vr`, hit-test sources, anchors, plane detection, CPU depth, inputs and hands; `deviceorientation` fallback         | Compiles for `wasm32-unknown-unknown`  |

Nothing here has run on a device: there was no phone, headset, or browser with WebXR in reach. The first device runs should check: ARKit's SIMD calls (below), ARCore's camera image strides and display transform on a rotated screen, the Android sensor queue made on the driving thread, OpenXR's session lifecycle on Quest, and WebXR's depth texture orientation.

### ARKit's SIMD methods

`objc2-ar-kit` leaves out every method that takes or returns `simd_float4x4` or `simd_float3`. On arm64 those are homogeneous vector aggregates passed in `v0`–`v3`; a Rust `#[repr(C)]` array of `float32x4_t` has the same classification, so `arkit/simd.rs` calls `objc_msgSend` cast to each exact signature. This is the part most worth testing first on a device; a wrong ABI shows up as garbage poses, not a crash.

### ARCore's camera picture

wgpu has no external textures (`samplerExternalOES`) and no sampler YCbCr conversion, so neither of ARCore's zero-copy outputs (the OES texture, the `AHardwareBuffer`) can be sampled from WGSL. The 3D renderer defaults to OpenGL ES on Android because the Solana Seeker's Vulkan driver reset the GPU when the camera closed. The backend therefore runs ARCore in `EXPOSE_HARDWARE_BUFFER` mode, which frees `ArSession_update` from needing a current GL context (so tracking runs on any thread), and copies `ArFrame_acquireCameraImage`'s YUV planes into luma and interleaved chroma, about 1.5 bytes per pixel (3 MB a frame at 1080p, a few milliseconds of memcpy on current phones, not measured here). The next step, once wgpu can import external textures on GL ES, is to wrap the hardware buffer in an `EGLImage` and skip the copy.

### Headsets

OpenXR needs the graphics device to make a session, and paces frames with `xrWaitFrame`. So `OpenXrBackend` only loads the instance and reports capabilities; a renderer installs a `SessionFactory` (`with_renderer`), and `render::openxr_vulkan::factory` makes the Vulkan instance and device through `XR_KHR_vulkan_enable2`, adopts them in wgpu through `wgpu-hal`, and adopts each swapchain image (two array layers, one per eye). Each `XrRuntime::update` on the render thread waits for the headset's frame and locates everything at its predicted display time; `VulkanHeadset::render` draws the eyes and ends the frame with the projection layer over Meta's passthrough layer when there is one. A skipped frame is ended empty on the next update, so `xrBeginFrame`/`xrEndFrame` always pair. GL ES sessions on headsets (`XR_KHR_opengl_es_enable`) are not wired.

### The browser

WebXR is reached through `Reflect` rather than `web-sys`'s WebXR bindings, which need `--cfg=web_sys_unstable_apis`. An immersive session runs no frames without a base layer, so the backend gives it an `XRWebGLLayer` on its own WebGL 2 canvas; a renderer replaces it through `Session::xr_session()`. WebXR hit tests resolve on the next frame, so `hit_test` waits one frame.

## The Quest target

`--platform quest` (`Platform::Quest`, `Target::Quest`, `NativeTarget::Quest`) builds the Android host: same `hosts/android` Gradle project, same `.leptoskit/android` output, `aarch64-linux-android`, and the guest built with the `android` feature (Horizon OS is Android, so `Platform::Quest.feature()` is `android`). `native/quest.rs` rewrites the merged manifest: head tracking, OpenXR permissions and the runtime broker queries, hand tracking and passthrough per `[native.quest]`, `com.oculus.supportedDevices`, a full-screen landscape `singleTask` activity without picture-in-picture, and the `com.oculus.intent.category.VR` and `IMMERSIVE_HMD` launcher categories. `leptoskit dev --platform quest` installs onto adb devices whose `ro.product.manufacturer` is Oculus or Meta. The manifest is snapshot-tested; nothing has been installed on a headset.

Android XR would be a small delta (its own `uses-feature` and activity property) but has no target yet. In immersive mode the Android view hierarchy is not composited, so the guest's 2D interface isn't visible; `immersive = false` gives a panel app.

## With `leptoskit_3d`

`leptoskit_3d` draws a `RenderTarget`: texture views, views with `Mat4` view and projection matrices, a `Background::Camera(CameraImage { planes, transform: Mat3 })`, and an `Occlusion { depth, transform, scale, tolerance }` reading an `R32Float` or `R16Float` texture. The `wgpu` feature produces exactly those inputs: `render::FrameTextures` uploads camera planes as `R8Unorm`/`Rg8Unorm` (or `Rgba8UnormSrgb`) and depth as `R32Float` metres, and `render::affine_matrix` turns a display transform into the column-major 3 × 3 matrix `Mat3::from_cols_array` takes.

The `3d` feature (`wgpu`, `leptoskit_3d` with its renderer, and `glam`) adds `host::scene3d::draw`, which does that mapping: the frame's views become `ViewParams` with a 5 cm near plane and a 100 m far plane, the camera picture becomes the background (or `Background::Scene` when there is none), and real-world depth becomes an `Occlusion` with 5 cm of tolerance for the sensor's noise. On a phone, `color` is `TargetColor::View` of the surface texture; on a headset it is `TargetColor::Layers` of the two eye views `VulkanHeadset::render` passes, with each view's viewport the eye's full size.

Native hosts don't call it yet: the iOS and Android hosts draw a `Scene3d` without the camera picture behind it, and handing a session's frame to the scene's renderer is the next step there.

## Not covered

Apple Vision Pro, image and object tracking, room meshes (ARKit scene reconstruction, Quest's scene API), shared or persistent anchors, the light probe's environment map, ARCore's environmental HDR main light, and zero-copy camera import on Apple (`CVMetalTextureCache`) and Android (`AHardwareBuffer`). `FrameTextures` copies ARKit's pixel buffer planes today.
