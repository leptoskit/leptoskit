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

## Estimated depth

Phones without platform depth (the Solana Seeker, which has no ARCore; ARCore phones without the Depth API; iPhones without LiDAR) can still have depth: [`leptoskit_depth`](../../crates/leptoskit_depth) reads it from one camera picture with Depth Anything V2 Small, the authors' metric indoor fine-tune, run through tract. The crate does not depend on `leptoskit_xr`, so `leptoskit_xr` can depend on it.

### What the crate gives

- `DepthModel::from_bytes(&[u8])` loads the ONNX file (0.5 s on an M4); `estimate(&Picture) -> DepthMap`. A `Picture` is `Pixels` (`Rgb8`, `Rgba8`, `Bgra8`, or `YCbCr` NV12 planes holding `Arc<[u8]>`, so `ImagePlane` data passes without a copy), an `Orientation` (the clockwise quarter turns to upright; `Orientation::from_display_transform` reads it from a view-to-picture affine), and optional `Intrinsics { focal, principal, size }`, the same fields as `host::Intrinsics` on the camera branch.
- The `DepthMap` lies the way the picture did and covers all of it at the model's resolution (a 4:3 sensor picture gives 448 × 336), zero outside the 9:16 upright crop the model reads. Values are metres along the camera's forward axis, the convention `DepthImage` documents, so `to_le_bytes()` is `DepthFormat::Meters32` with `row_stride = width * 4`, and the picture's own display transform serves it unchanged.
- The model reads a 252-pixel-wide picture about 45% long; `estimate` divides out Reapers' fitted curve, `overreach(f) = 1.026 · (579 / f)^0.48` for the focal length `f` in input pixels (1.39 on the Seeker). Without intrinsics it assumes 68° across the longer side.
- `DepthWorker<T>` is the seam: `max_side()` (896: twice the model's long side), `ready()`, `submit(Frame { picture, time, context: T })` (drops the frame unless idle and a period, 1 s by default, has passed since the last accepted one), `latest() -> Option<Arc<Estimate<T>>>` (with a `sequence` that counts up), `last_error()`, and `reset()`, which also discards the picture in flight. None of them wait on the model; it runs on one `leptoskit-depth` thread, single-threaded.

### Why tract

Measured on an Apple M4 with the pinned file (`examples/benchmark.rs`, and a scratch harness for ONNX Runtime through `ort` 2.0.0-rc.15), warm medians on a 252 × 448 picture while the machine was otherwise quiet:

| Runtime                         | 1 thread | 2 threads | 4 threads | Load  |
| ------------------------------- | -------- | --------- | --------- | ----- |
| tract 0.23.9                    | 264 ms   | 258 ms    | 247 ms    | 0.5 s |
| ONNX Runtime, optimisations off | 252 ms   | 191 ms    | 170 ms    | 3.7 s |

ONNX Runtime with its default graph optimisations (any level from basic up) returned a near-constant map on Apple silicon: wrong, not slow. With them off it agrees with tract to the centimetre on the fixtures. tract is 1.05× ONNX Runtime on one thread and 1.35× on two, inside the 2× budget, and needs no native library: `ort` downloads a prebuilt static ONNX Runtime per target (97 MB for `aarch64-apple-ios`, 136 MB for `aarch64-linux-android`, from pyke's CDN). tract's own threading barely helps this quantised model, so the crate doesn't enable it. tract-linalg compiles assembly kernels with the target's C compiler: `cargo build --target aarch64-linux-android` needs the NDK's clang (as `setup/scripts/clippy.sh` already exports), and with Apple's clang standing in it built a static library for both `aarch64-linux-android` and `aarch64-apple-ios`. Neither has run on a phone. Under load from other builds the same benchmark read 650–830 ms, so compare numbers taken together.

### The weights

The 27 MB file is not committed here (see the image rule: binaries are hosted). `MODEL` pins its SHA-256 (`b81c4fae…`) and size, and its `url` is the raw file at `pina-rs/reapers@7a10588e`, the commit that added it to the Flutter game. That commit is no longer on any branch of `pina-rs/reapers` (its history was rewritten), so GitHub may collect it one day: before relying on it, tag it there or upload the file to the `leptoskit` B2 bucket and repoint `MODEL.url`. The digest stays the same either way. `MODEL_LICENSE` is the Apache-2.0 text, which ships with the file. Apps put the file in `assets/` and the host reads its bytes through the app's asset source, as `leptoskit_3d` does. The CLI does not fetch it; a `leptoskit build` step that downloads `MODEL.url` into the build's assets and checks the digest would be the next step if apps should not carry it themselves.

### Wiring it into `leptoskit_xr`

`feat/xr-native-camera` adds the `host::DepthEstimator` trait, `host::CameraFrame` (an RGB `ImagePlane` shrunk to `max_side`, the viewer's `pose`, `Intrinsics`, the picture's `display_transform`, and `time`), and `XrRuntime::set_depth_estimator`. Its runtime already asks, for every frame of a session that wanted `DEPTH` and whose backend has none: `ready()`, then `submit(Arc<CameraFrame>)`, then sets `frame.depth = latest()`. Once that branch is on `main`, the follow-up is:

1. In `leptoskit_xr/Cargo.toml`, a `depth-estimation = ["host", "dep:leptoskit_depth"]` feature with `leptoskit_depth = { workspace = true, optional = true }`.
2. A `host/estimated_depth.rs` that adapts the worker, about forty lines:

   ```rust
   pub struct EstimatedDepth {
   	worker: leptoskit_depth::DepthWorker<Captured>,
   	converted: Option<(u64, DepthImage)>,
   }

   struct Captured {
   	pose: Pose,
   	display_transform: Affine2,
   }

   impl DepthEstimator for EstimatedDepth {
   	fn max_side(&self) -> u32 {
   		self.worker.max_side()
   	}

   	fn ready(&self) -> bool {
   		self.worker.ready()
   	}

   	fn submit(&mut self, frame: Arc<CameraFrame>) {
   		let rgb = &frame.rgb;
   		let plane = leptoskit_depth::Plane {
   			width: rgb.width,
   			height: rgb.height,
   			row_stride: rgb.row_stride,
   			data: Arc::clone(&rgb.data),
   		};
   		let picture = leptoskit_depth::Picture::new(leptoskit_depth::Pixels::Rgb8(plane))
   			.with_orientation(leptoskit_depth::Orientation::from_display_transform(&frame.display_transform))
   			.with_intrinsics(leptoskit_depth::Intrinsics {
   				focal: frame.intrinsics.focal,
   				principal: frame.intrinsics.principal,
   				size: frame.intrinsics.size,
   			});
   		self.worker.submit(leptoskit_depth::Frame {
   			picture,
   			time: frame.time,
   			context: Captured { pose: frame.pose, display_transform: frame.display_transform },
   		});
   	}

   	fn latest(&mut self) -> Option<DepthImage> {
   		let estimate = self.worker.latest()?;
   		if self.converted.as_ref().map(|(sequence, _)| *sequence) != Some(estimate.sequence) {
   			let map = &estimate.depth;
   			let image = DepthImage {
   				width: map.width,
   				height: map.height,
   				format: DepthFormat::Meters32,
   				data: map.to_le_bytes().into(),
   				row_stride: map.width * 4,
   				confidence: None,
   				display_transform: estimate.context.display_transform,
   			};
   			self.converted = Some((estimate.sequence, image));
   		}
   		self.converted.as_ref().map(|(_, image)| image.clone())
   	}

   	fn reset(&mut self) {
   		self.worker.reset();
   		self.converted = None;
   	}
   }
   ```

   Converting once per `sequence` keeps the per-frame cost to an `Arc` clone.
3. In `host::from_context`, a `[native.plugin-options.xr] depth-model = "depth/depth_anything_v2_metric_indoor_small.onnx"` option (an asset path; the schema and `leptoskit_config` gain it). When it is set, read the bytes through the app's asset source on a background thread (loading takes 0.5–1.5 s; plugin creation must not wait), `MODEL.verify` them when the file is meant to be the pinned one, `DepthModel::from_bytes`, `DepthWorker::spawn`, and `runtime.set_depth_estimator(Some(Box::new(EstimatedDepth::new(worker))))`. Until it lands, `capabilities()` doesn't offer estimated `DEPTH`, which is honest.
4. The depth is up to a second and a model run old, and the runtime hands it out with the current frame's views, so `hit_test::hit_depth` would cast against the room as seen from where the phone is now. Keep the capture pose (`Captured::pose`) and test against a `View { pose: captured_pose, projection: <the frame's view projection> }`: in an orientation session the projection never changes, and in 3DoF the viewer never moves, so that is exact up to tracking drift. The cleanest way is an `XrFrame::depth_view: Option<View>` that `Backend::hit_test`'s default and `scene3d::draw` prefer over `views[0]` when set; occlusion can then reproject the stale depth by rotation alone.
5. Tests: a replay frame with a camera picture of a fixture and a stub `Estimator` (the trait `leptoskit_depth` exports for tests) through `XrRuntime`, checking that `frame.depth` appears, that `DEPTH` hit tests land at the stub's distance from the capture pose, and that `stop` resets the worker. The real model belongs in `leptoskit_depth`'s own tests.

On a device, first check: the time per picture on the Seeker's cores (the Flutter game measured 0.5 s with ONNX Runtime on two), that the game still draws smoothly while the worker runs, and the scale against a measured floor.

## Not covered

Apple Vision Pro, image and object tracking, room meshes (ARKit scene reconstruction, Quest's scene API), shared or persistent anchors, the light probe's environment map, ARCore's environmental HDR main light, and zero-copy camera import on Apple (`CVMetalTextureCache`) and Android (`AHardwareBuffer`). `FrameTextures` copies ARKit's pixel buffer planes today.
