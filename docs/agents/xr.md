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
| Orientation | iOS (Core Motion), Android (NDK game rotation vector) | 3DoF with recentred heading and screen rotation; on Android, the back camera through Camera2 (`camera2.rs`)                         | Math tested; sources compile-only      |
| ARKit       | iOS arm64                                             | World tracking, planes with labels and outlines, smoothed LiDAR depth and confidence, light, raycasts, anchors                      | Compiles for `aarch64-apple-ios(-sim)` |
| ARCore      | Android                                               | Planes, Depth API, ambient light, native hit tests, anchors, install and permission checks; `libarcore_sdk_c.so` opened at run time | Compiles for `aarch64-linux-android`   |
| OpenXR      | Android (Quest, Android XR), Windows, Linux           | Instance, stereo views, Touch and simple controllers, `XR_EXT_hand_tracking`, `XR_FB_passthrough`, floor spaces                     | Compiles for `aarch64-linux-android`   |
| WebXR       | browser (`web` feature)                               | `immersive-ar`/`-vr`, hit-test sources, anchors, plane detection, CPU depth, inputs and hands; `deviceorientation` fallback         | Compiles for `wasm32-unknown-unknown`  |

Nothing here has run on a device: there was no phone, headset, or browser with WebXR in reach. The first device runs should check: ARKit's SIMD calls (below), ARCore's camera image strides and display transform on a rotated screen, the Android sensor queue made on the driving thread, Camera2's sensor orientation and stream size on the Seeker, the Android `SurfaceView` restacking when the camera starts, OpenXR's session lifecycle on Quest, and WebXR's depth texture orientation.

### ARKit's SIMD methods

`objc2-ar-kit` leaves out every method that takes or returns `simd_float4x4` or `simd_float3`. On arm64 those are homogeneous vector aggregates passed in `v0`–`v3`; a Rust `#[repr(C)]` array of `float32x4_t` has the same classification, so `arkit/simd.rs` calls `objc_msgSend` cast to each exact signature. This is the part most worth testing first on a device; a wrong ABI shows up as garbage poses, not a crash.

### ARCore's camera picture

wgpu has no external textures (`samplerExternalOES`) and no sampler YCbCr conversion, so neither of ARCore's zero-copy outputs (the OES texture, the `AHardwareBuffer`) can be sampled from WGSL. The 3D renderer defaults to OpenGL ES on Android because the Solana Seeker's Vulkan driver reset the GPU when the camera closed. The backend therefore runs ARCore in `EXPOSE_HARDWARE_BUFFER` mode, which frees `ArSession_update` from needing a current GL context (so tracking runs on any thread), and copies `ArFrame_acquireCameraImage`'s YUV planes into luma and interleaved chroma, about 1.5 bytes per pixel (3 MB a frame at 1080p, a few milliseconds of memcpy on current phones, not measured here). The next step, once wgpu can import external textures on GL ES, is to wrap the hardware buffer in an `EGLImage` and skip the copy.

### The camera without ARCore

Phones without ARCore (the Solana Seeker among them) run `Orientation` sessions from the rotation sensor. On Android, `OrientationBackend::with_camera(Camera2Source)` adds the back camera, so a scene still draws over the room. `camera2.rs` opens `libcamera2ndk.so` and `libmediandk.so` at run time, like ARCore, picks the first back camera with a YUV stream, chooses the largest YUV size of at most 1280 × 960 nearest the sensor's aspect, and polls an `AImageReader` (two images, no listener or looper) with `acquireLatestImage`. Each new image is copied into luma and interleaved chroma with the same `ycbcr_from_yuv420` ARCore's CPU image goes through.

The picture's orientation comes from `SENSOR_ORIENTATION` and the interface's rotation (`upright_transform`: the back camera turns `(sensor − display) mod 4` quarters clockwise, then the turned picture is cropped to fill the view), and the session's field of view from the camera's focal length over the cropped picture (`upright_fov`), so the scene keeps the picture's scale. The focal length is `LENS_INTRINSIC_CALIBRATION` where it is non-zero, otherwise the lens's focal length over the sensor's pixel pitch, scaled to the stream (a stream is the active array's centre cropped to its aspect).

The camera opens only for sessions that ask for `CAMERA_IMAGE`. Without the permission Camera2 refuses to open; the session then starts without the picture, unless it required it, so the rotation fallback keeps working for apps that never ask for the camera. Every iPhone that runs current iOS has ARKit, so iOS has no camera-only source.

### Depth from the camera picture

`host::camera` is the seam a monocular depth model plugs into. `XrFrame::camera_intrinsics` describes the camera picture's projection (ARKit's `ARCamera.intrinsics`, ARCore's `ArCamera_getImageIntrinsics`, Camera2's characteristics). `CameraFrame::from_frame(frame, max_side)` shrinks a frame's picture to sRGB RGB on the CPU (a box filter on luma), with its time, the frame's first `View` (pose and projection), the scaled intrinsics, and the picture's display transform. A `DepthEstimator` installed with `XrRuntime::set_depth_estimator`:

- makes the runtime offer `DEPTH` in every mode that offers `CAMERA_IMAGE`;
- for a session that gets `DEPTH` but whose backend has none (the rotation fallback, ARCore without the Depth API), has the backend open the camera even when the app didn't ask for the picture, and is offered a `CameraFrame` on the driving thread whenever it says it is `ready`;
- answers each frame with `latest()`, a `DepthEstimate`: the newest depth it finished (metres or millimetres along the camera's axis, in the display transform of the frame it came from), with that frame's `view` and `time`. The runtime makes them `XrFrame::depth` and `XrFrame::depth_view`.

The generated hosts make the plugin's runtime themselves, so a depth crate calls `host::install_depth_estimator(make)` once: every runtime the plugin has made, and every one it makes later, gets an estimator from `make`.

Every call returns at once; a model runs on its own thread and drops frames it can't keep up with.

The depth can be a second and a model run old, so it is served with the view it was seen from. `XrFrame::depth_view` is `Some` only for estimated depth; platform depth leaves it `None` and behaves exactly as before.

- **Hit tests.** `XrRuntime::hit_test` handles `DEPTH` itself when `depth_view` is set: the ray still comes from the current `views[0]` (a tap is on today's screen), and `hit_test::hit_depth` marches it through the old depth from `depth_view`. The backend answers the other targets with `DEPTH` taken out of the request, so ARKit's and ARCore's own hit tests, which never see estimated depth, are unchanged. In a 3DoF session the viewer never moves and the projection never changes, so this is exact up to the sensor's drift; in 6DoF it is exact for a still room.
- **Occlusion.** `XrFrame::depth_transform` is the depth's display transform after `hit_test::view_to_view(views[0], depth_view)`, the turn between the views as an affine: the tangent of the true perspective map at the view's centre, fitted to the centre and the midpoints of the edges. On a 1080 × 1920 portrait view with a 60° vertical field, a 10° turn leaves the corners 60 px (yaw) to 120 px (pitch) out, against 350 to 440 px with no reprojection; a least-squares fit over the whole view would trade the exact centre for about 25% less at the corners. It ignores translation (right for an orientation session) and compares depth along the old camera's axis with the fragment's along the new one, which differ by the cosine of the turn. `scene3d::draw` uses it for estimated depth only, and draws without occlusion when the views face more than about 90° apart. A perspective-correct reprojection would need `leptoskit_3d`'s occlusion to take a 3 × 3 homography, which its shader could do with one more divide.

### Headsets

OpenXR needs the graphics device to make a session, and paces frames with `xrWaitFrame`. So `OpenXrBackend` only loads the instance and reports capabilities; a renderer installs a `SessionFactory` (`with_renderer`), and `render::openxr_vulkan::factory` makes the Vulkan instance and device through `XR_KHR_vulkan_enable2`, adopts them in wgpu through `wgpu-hal`, and adopts each swapchain image (two array layers, one per eye). Each `XrRuntime::update` on the render thread waits for the headset's frame and locates everything at its predicted display time; `VulkanHeadset::render` draws the eyes and ends the frame with the projection layer over Meta's passthrough layer when there is one. A skipped frame is ended empty on the next update, so `xrBeginFrame`/`xrEndFrame` always pair. GL ES sessions on headsets (`XR_KHR_opengl_es_enable`) are not wired.

### The browser

WebXR is reached through `Reflect` rather than `web-sys`'s WebXR bindings, which need `--cfg=web_sys_unstable_apis`. An immersive session runs no frames without a base layer, so the backend gives it an `XRWebGLLayer` on its own WebGL 2 canvas; a renderer replaces it through `Session::xr_session()`. WebXR hit tests resolve on the next frame, so `hit_test` waits one frame.

## The Quest target

`--platform quest` (`Platform::Quest`, `Target::Quest`, `NativeTarget::Quest`) builds the Android host: same `hosts/android` Gradle project, same `.leptoskit/android` output, `aarch64-linux-android`, and the guest built with the `android` feature (Horizon OS is Android, so `Platform::Quest.feature()` is `android`). `native/quest.rs` rewrites the merged manifest: head tracking, OpenXR permissions and the runtime broker queries, hand tracking and passthrough per `[native.quest]`, `com.oculus.supportedDevices`, a full-screen landscape `singleTask` activity without picture-in-picture, and the `com.oculus.intent.category.VR` and `IMMERSIVE_HMD` launcher categories. `leptoskit dev --platform quest` installs onto adb devices whose `ro.product.manufacturer` is Oculus or Meta. The manifest is snapshot-tested; nothing has been installed on a headset.

Android XR would be a small delta (its own `uses-feature` and activity property) but has no target yet. In immersive mode the Android view hierarchy is not composited, so the guest's 2D interface isn't visible; `immersive = false` gives a panel app.

### Immersive sessions from a panel

Horizon OS shows an OpenXR session only from an activity in the `com.oculus.intent.category.VR` category, and Meta's route for an app that is both (a "hybrid app", Horizon OS v69 and later) is two activities: a 2D panel launcher in `com.oculus.intent.category.2D`, and an immersive activity the panel starts with an explicit `ACTION_MAIN` intent and `FLAG_ACTIVITY_NEW_TASK`. Going back, the immersive activity sends a `CATEGORY_HOME` intent carrying a `PendingIntent` for the panel in `extra_launch_in_home_pending_intent`, so the panel opens in Home, and finishes. Meta's Spatial SDK `HybridSample` does exactly this, and `quest.rs` follows its manifest: every Quest build declares `dev.leptoskit.mobile.NativeImmersiveActivity` (`mobile/NativeImmersive.kt`) in the VR category, not exported and not a launcher; a panel app's launcher also gets the `2D` and `DEFAULT` categories and `<horizonos:uses-horizonos-sdk>` at 69. Unlike the sample, the panel is not finished when the immersive activity opens, because the guest runs in it.

The `quest` feature (`openxr` and `scene3d`; `leptoskit build --platform quest` turns on every async plugin's `quest` feature, `registry::plugin_features`) gives the OpenXR backend `host::immersive::Immersive::factory` as its renderer. Starting an `ImmersiveVr` or `ImmersiveAr` session then:

1. calls `NativeImmersive.open` through the app's class loader (the render thread's own can't see app classes), which fails the start with `Unavailable` where the activity isn't declared;
2. returns a `Pending` headset, so `start` answers at once and the guest's frame requests simply wait;
3. on its own thread, waits up to ten seconds for the activity, makes the Vulkan device and session with `openxr_vulkan::factory` on the instance the backend already loaded (with the panel's activity: OpenXR allows one instance, and which activity it names has not been checked on a headset), hands the real session to `Pending`, and becomes the runtime's driver (`set_external_driver`);
4. each frame, `XrRuntime::update` (which waits for the headset) and `VulkanHeadset::render`, which no longer holds the session's lock while drawing, so drawing may lock the runtime and the scene; it draws the scene the guest chose with `Session::present` through `Hub::draw_scene` and `scene3d::draw` into both eye layers, and clears them to transparent (passthrough, or black) when there is none;
5. ends when the guest stops the session, the session ends (the runtime is then stopped, so the guest's frame requests fail with `Cancelled`), or the activity is gone; it moves the scenes off the headset's device (`Hub::forget_device`), drops the session before the device, and calls `NativeImmersive.close`.

A scene lives in one renderer at a time; `Hub::draw_scene` and a view's frame move it to their own device when it is on another, and its assets upload again there.

## With `leptoskit_3d`

`leptoskit_3d` draws a `RenderTarget`: texture views, views with `Mat4` view and projection matrices, a `Background::Camera(CameraImage { planes, transform: Mat3 })`, and an `Occlusion { depth, transform, scale, tolerance }` reading an `R32Float` or `R16Float` texture. The `wgpu` feature produces exactly those inputs: `render::FrameTextures` uploads camera planes as `R8Unorm`/`Rg8Unorm` (or `Rgba8UnormSrgb`) and depth as `R32Float` metres, and `render::affine_matrix` turns a display transform into the column-major 3 × 3 matrix `Mat3::from_cols_array` takes.

The `3d` feature (`wgpu`, `leptoskit_3d` with its renderer, and `glam`) adds `host::scene3d::draw`, which does that mapping: the frame's views become `ViewParams` with a 5 cm near plane and a 100 m far plane, the camera picture becomes the background (or `Background::Scene` when there is none), and real-world depth becomes an `Occlusion` with 5 cm of tolerance for the sensor's noise. On a phone, `color` is `TargetColor::View` of the surface texture; on a headset it is `TargetColor::Layers` of the two eye views `VulkanHeadset::render` passes, with each view's viewport the eye's full size.

### Phones

The `scene3d` feature (`3d`, `async-host`, and `leptoskit_3d/host`) wires it into the native hosts. `leptoskit_3d::host::Hub` takes a `CameraFeed`; a `View` whose scene's backdrop is `Backdrop::Camera` asks the feed for a `CameraPainter` and lets it draw the frame, or draws as before when the painter has no picture. `SessionFeed::install`, called from `from_context`, makes the plugin's runtime the global hub's feed and sets the runtime's listener to wake every view when a session starts, changes, or stops. A painter:

- tells the runtime the view's size and rotation as its `Viewport` (on change, and before a session starts, so ARKit's display transform and projection and ARCore's display geometry fit the view from the first frame);
- draws only `PhoneAr` and `Orientation` sessions, never a headset's;
- takes the runtime's latest frame (the driver thread keeps updating it), uploads its pictures into the view's own `FrameTextures` once per frame time, and drops the frame, so ARKit gets its pixel buffers back; `FrameTextures` also skips a picture whose buffer it already holds (a 30 Hz camera under a 60 Hz rotation sensor);
- draws through `draw`, so the frame's view replaces the scene's camera.

A view keeps drawing on every display frame while the feed is live. The Swift view reports its window scene's interface orientation each frame (a half turn between landscapes changes no size), and the Kotlin view `Display.getRotation()`, through `lk_scene3d_view_set_rotation`. The Android `SurfaceView` sits on the media overlay layer so a transparent scene shows the window behind it; once `lk_scene3d_view_shows_camera` says it drew the camera, it leaves its parent and comes back in the same place below the window, since a `SurfaceView`'s layer only changes while detached, so the interface laid over the scene stays visible.

`leptoskit build` turns on the `scene3d` feature of every selected async plugin that has one when the app shows 3D scenes (`registry::scene3d_features`), next to the bridge's own.

## Estimated depth

Phones without platform depth (the Solana Seeker, which has no ARCore; ARCore phones without the Depth API; iPhones without LiDAR) can still have depth: [`leptoskit_depth`](../../crates/leptoskit_depth) reads it from one camera picture with Depth Anything V2 Small, the authors' metric indoor fine-tune, run through tract. The crate does not depend on `leptoskit_xr`, so `leptoskit_xr` can depend on it.

### What the crate gives

- `DepthModel::from_bytes(&[u8])` loads the ONNX file (0.5 s on an M4); `estimate(&Picture) -> DepthMap`. A `Picture` is `Pixels` (`Rgb8`, `Rgba8`, `Bgra8`, or `YCbCr` NV12 planes holding `Arc<[u8]>`, so `ImagePlane` data passes without a copy), an `Orientation` (the clockwise quarter turns to upright; `Orientation::from_display_transform` reads it from a view-to-picture affine), and optional `Intrinsics { focal, principal, size }`, the same fields as `host::Intrinsics` on the camera branch.
- The `DepthMap` lies the way the picture did and covers all of it at the model's resolution (a 4:3 sensor picture gives 448 × 336), zero outside the 9:16 upright crop the model reads. Values are metres along the camera's forward axis, the convention `DepthImage` documents, so `to_le_bytes()` is `DepthFormat::Meters32` with `row_stride = width * 4`, and the picture's own display transform serves it unchanged.
- The model reads a 252-pixel-wide picture about 45% long; `estimate` divides out Reapers' fitted curve, `overreach(f) = 1.026 · (579 / f)^0.48` for the focal length `f` in input pixels (1.39 on the Seeker). Without intrinsics it assumes 68° across the longer side.
- `DepthWorker<T>` is the seam: `max_side()` (896: twice the model's long side), `ready()`, `submit(Frame { picture, time, context: T })` (drops the frame unless idle and a period, 1 s by default, has passed since the last accepted one), `latest() -> Option<Arc<Estimate<T>>>` (with a `sequence` that counts up), `last_error()`, and `reset()`, which also discards the picture in flight. None of them wait on the model; it runs on one `leptoskit-depth` thread, single-threaded.

### Why tract

Measured on an Apple M4 with the pinned file (`examples/benchmark.rs`, and a scratch harness for ONNX Runtime through `ort` 2.0.0-rc.15), warm medians of 7 runs on a 252 × 448 picture with the machine lightly loaded (load average about 5):

| Runtime                         | 1 thread | 2 threads | 4 threads | Load  |
| ------------------------------- | -------- | --------- | --------- | ----- |
| tract 0.23.9                    | 264 ms   | 258 ms    | 247 ms    | 0.5 s |
| ONNX Runtime, optimisations off | 252 ms   | 191 ms    | 170 ms    | 3.7 s |

ONNX Runtime with its default graph optimisations (any level from basic up) returned a near-constant map on Apple silicon: wrong, not slow. With them off it agrees with tract to the centimetre on the fixtures. tract is 1.05× ONNX Runtime on one thread and 1.35× on two, inside the 2× budget, and needs no native library: `ort` downloads a prebuilt static ONNX Runtime per target (97 MB for `aarch64-apple-ios`, 136 MB for `aarch64-linux-android`, from pyke's CDN). tract's own threading barely helps this quantised model, so the crate doesn't enable it. tract-linalg compiles assembly kernels with the target's C compiler: `cargo build --target aarch64-linux-android` needs the NDK's clang (as `setup/scripts/clippy.sh` already exports), and with Apple's clang standing in it built a static library for both `aarch64-linux-android` and `aarch64-apple-ios`. Neither has run on a phone. With other agents' builds running (load average 15 to 20) every runtime read 450 ms to 1 s and their order was not stable, so compare numbers taken together.

### The weights

The 27 MB file is not committed here (see the image rule: binaries are hosted). `MODEL` pins its SHA-256 (`b81c4fae…`) and size, and its `url` is the `depth-model-v1` release asset on `pina-rs/reapers`, where the Reapers game, which first shipped the file, keeps it beside its licence. Moving it (to the `leptoskit` B2 bucket, say) only changes `MODEL.url`; the digest stays the same. `MODEL_LICENSE` is the Apache-2.0 text, which ships with the file. Apps put the file in `assets/` and name it in `depth-model`, and the plugin reads its bytes from the app bundle (below). The CLI does not fetch it; a `leptoskit build` step that downloads `MODEL.url` into the build's assets and checks the digest would be the next step if apps should not carry it themselves.

### How `leptoskit_xr` runs it

- **The feature.** `depth-estimation = ["host", "dep:leptoskit_depth"]`; the dependency is native only, so `lint:clippy:wasm`'s `--all-features` build of `leptoskit_xr` doesn't pull tract into wasm.
- **The adapter.** `host::ModelEstimator` (`estimated_depth.rs`) is the `DepthEstimator` over an `Arc<DepthWorker<Captured>>`, where `Captured` keeps the frame's `View` and display transform. `submit` builds a `leptoskit_depth::Picture` that shares the `CameraFrame`'s `Arc<[u8]>` (no copy), with `Orientation::from_display_transform` and the scaled intrinsics. `latest` converts each estimate once per `sequence` into a `DepthEstimate` (`Meters32`, `row_stride = width * 4`, the captured display transform), so every other frame costs an `Arc` clone. Clones share the worker; `install_depth_model(model)` spawns one and installs `move || Box::new(estimator.clone())` through `install_depth_estimator`, so every runtime the plugin makes (there is one per process in practice, more across guest reloads) shares one model and one thread, and a reset from one clears it for all.
- **The option.** `[native.plugin-options.xr] depth-model = "<path in assets/>"` is parsed whether or not the feature is on (the options are `deny_unknown_fields`); without the feature `from_context` writes a warning to stderr and carries on. With it, `load_in_background` starts a `leptoskit-xr-depth-model` thread: `bundle::check_path` (relative, `/`-separated, no empty, `.` or `..` segments), `bundle::read` (Android: `AssetManager.open("assets/<path>")` over JNI on a scoped attach, 1 MiB at a time, capped at 64 MiB; elsewhere `assets/` beside the executable, which on iOS is the app bundle's own), `MODEL.verify`, `DepthModel::from_bytes`, `install_depth_model`. Every failure is logged (`__android_log_write` with tag `leptoskit-xr` on Android, stderr elsewhere) and leaves the runtime without an estimator, so `capabilities()` doesn't offer estimated `DEPTH`. A session started before the model loads keeps going without it (`set_depth_estimator` applies from the next session). Plugin options are free-form tables, so neither `leptoskit_config` nor the schema changed.
- **The build.** Plugin metadata gained `option-features`, a map from option name to a feature of the plugin's crate. `configure_plugin` records the features whose options the app set, and `host_features` adds `<key>/<feature>` next to `<key>/async-host`, failing when the package has no such feature. `leptoskit_xr` declares `option-features = { depth-model = "depth-estimation" }`, so tract is compiled only into apps that set the option.
- **Asset reading.** `leptoskit_3d`'s `Hub` asset source is set by the Swift and Kotlin `Scene3d` views the first time one attaches, too late and too conditional for a model the plugin wants at start, and `leptoskit_native::asset::bundled_folder` would pull the component crate into `leptoskit_xr`. `bundle.rs` follows the audio plugin's `platform::asset` instead.

Apps build tract in their own workspace, so its `[profile.dev.package.tract-*] opt-level = 3` overrides must be in the app's root `Cargo.toml` too, or a debug build takes about 19 s a picture (the AR guide says so).

### What is tested, and what isn't

- `tests/camera.rs`: an estimator fills in depth on an orientation session (`depth_view` is the frame's own view, `depth_transform` the plain display transform); on a 6DoF backend without depth (`Retreat`, like ARCore without the Depth API) a `DEPTH` hit test after the viewer backed a metre away lands three metres off, where the old picture put the wall, not two; `stop` resets the estimator and the next session reads its own; a backend that measures depth keeps it, the estimator is never asked, and `depth_view` stays `None`.
- `estimated_depth.rs`: frames reach a stub `leptoskit_depth::Estimator` turned clockwise with the frame's focal length and the same pixel buffer, come back as `Meters32` with the captured view and display transform, are converted once, shared by clones, and forgotten on reset. `bundle.rs`: path checks and the missing-file message. `hit_test.rs`: `view_to_view` is the identity for one view, puts straight ahead where a turned view sees it, and refuses a view facing away. `tests/plugin.rs`: a missing or escaping `depth-model` never stops the plugin.
- `tests/depth_model.rs` runs the real model through `XrRuntime` when `LEPTOSKIT_DEPTH_MODEL` names the file: `leptoskit_depth`'s corridor fixture as an RGBA camera picture with a 300-pixel focal length gave depth after 1.3 s, and screen-tap hit tests read the floor at 1.10 m and the boiler at 1.53 m along their rays.
- `leptoskit_xr` lints clean for the host, `aarch64-apple-ios`, `aarch64-linux-android` (with Apple's clang standing in for the NDK's), and wasm. Neither the JNI asset read, the iOS bundle path, nor the model has run on a phone.

On a device, first check: that the model loads from the APK and the iOS bundle (logcat's `leptoskit-xr` tag says which), the time per picture on the Seeker's cores (the Flutter game measured 0.5 s with ONNX Runtime on two), that the game still draws smoothly while the worker runs, occlusion while turning, and the scale against a measured floor.

## Not covered

Running any of the Quest path on a headset: the hybrid manifest, the activity switch and the return to Home, the session on the panel's OpenXR instance, and whether the guest keeps receiving frames while its panel is stopped. Apple Vision Pro, image and object tracking, room meshes (ARKit scene reconstruction, Quest's scene API), shared or persistent anchors, the light probe's environment map, ARCore's environmental HDR main light, and zero-copy camera import on Apple (`CVMetalTextureCache`) and Android (`AHardwareBuffer`). `FrameTextures` copies ARKit's pixel buffer planes today. Estimated depth is not reprojected for translation, and `leptoskit build` does not fetch the model: an app ships it. Two `Scene3d` views of different sizes showing the camera at once share one viewport, the last one drawn.
