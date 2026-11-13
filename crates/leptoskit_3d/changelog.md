# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Textured models packed by gltfpack.** Scenes read `KHR_texture_transform`, so a textured model compressed by `gltfpack` draws its textures where they belong. gltfpack quantizes texture coordinates and records the scale and offset that undo it as a texture transform; scenes used to ignore it, and every texture was sampled from its corner. The transform is applied to a primitive's texture coordinates as the model loads. A material's textures share one transform, the first found starting from the base color texture, which is what gltfpack writes. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #713](https://github.com/leptoskit/leptoskit/pull/713)
- **Deliver primary-pointer drags, taps and cancellations from Android, iOS and macOS scene views through `Scene::next_pointer`.** Movement uses layout units and a bounded queue coalesces drags independently of model-loading events. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **Additive light for glows, flashes, and sparks.** `Scene::set_blend(node, part, Blend::Additive)` (`Op::SetBlend`) makes a part of a model add its colour to what is drawn behind it, scaled by its opacity, instead of covering it. A muzzle flash, a spark, or a soft halo then reads as light over a camera picture, and black adds nothing, so a round glow needs no cut-out. `Blend::Normal` puts it back. Additive parts draw with the transparent parts. The op is new, so batches without it encode exactly as before. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #714](https://github.com/leptoskit/leptoskit/pull/714)

#### Move a model's own parts from your app

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #712](https://github.com/leptoskit/leptoskit/pull/712)

`Scene::set_part_offset(node, part, offset, seconds)` (`Op::SetPartOffset`) moves a named part of a model instance, a bone or any node, from where its rest pose or the playing clip puts it. The offset turns the part about its own origin and moves it along its own axes, and everything under the part follows, skinned meshes included, so an app can turn a creature's head towards the camera while its walk clip plays, swing a rigid leg, or open a hinged lid by exactly as much as its own state says.

The host moves the part there over `seconds`, in a straight line from where it is now, on its own display clock. An app that works out a pose on each tick of its own clock and sends it with the tick's length is drawn smoothly between ticks. `Transform::IDENTITY` puts the part back. The op is new, so batches without it encode exactly as before.

#### 3D scenes over the live camera on iOS and Android

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #681](https://github.com/leptoskit/leptoskit/pull/681)

A `Scene3d` whose backdrop is `Backdrop::Camera` now draws over the camera while an `xr` session runs on a phone. The generated iOS and Android apps hand the scene the session's newest frame: the camera picture behind the models, turned upright for the screen's rotation and cropped to fill the view, the session's own viewpoint in place of the scene's camera, and the room's measured depth hiding the parts of the scene behind real things. Start the session with `Features::CAMERA_IMAGE` (and `Features::DEPTH` for occlusion). Without a session the scene draws as before, transparent behind its models.

Android phones without ARCore, such as the Solana Seeker, now show the back camera in `Orientation` sessions too, through Camera2, with the rotation sensor turning the view and the camera's own field of view keeping the scene to the picture's scale. Such a session leaves the picture out, rather than failing, until the user allows the camera, unless it requires `CAMERA_IMAGE`.

On Android a `Scene3d` showing the camera moves below the window's own views, so the interface laid over it stays visible.

For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::set_camera_feed` takes the picture a camera backdrop draws over, `View::set_rotation` the interface's rotation, and the bridge adds `lk_scene3d_view_set_rotation` and `lk_scene3d_view_shows_camera`. `leptoskit_xr`'s new `scene3d` feature makes the plugin's sessions that feed, and `leptoskit build` turns it on, with any other async plugin's `scene3d` feature, when the app shows 3D scenes. `leptoskit_xr::host::install_depth_estimator` (or `XrRuntime::set_depth_estimator`) takes a `DepthEstimator`, which turns small CPU copies of the camera picture (`CameraFrame`, with the camera's `Intrinsics`) into depth for phones that measure none.

#### Immersive sessions from Meta Quest panel apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #691](https://github.com/leptoskit/leptoskit/pull/691)

A Quest app now draws its 3D scene into the headset. Start an `ImmersiveVr` or `ImmersiveAr` session and choose the scene with the new `Session::present(Some(scene_id))`: each eye sees the scene from the player's eye, over passthrough in `ImmersiveAr`. An app built with `[native.quest] immersive = false`, a 2D panel, opens an immersive activity over the panel for the session and comes back to the panel in Horizon Home when the session ends, while its code keeps running and receiving frames.

`leptoskit build --platform quest` declares that activity in every Quest app, marks a panel app's launcher with Horizon OS's 2D category and targets Horizon OS v69, and turns on `leptoskit_xr`'s new `quest` feature (or any async plugin's `quest` feature). For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::draw_scene` draws a scene on another device, such as a headset's, and `Hub::forget_device` moves scenes off it again.

None of this has run on a headset yet.

### Fixes

- **Transparent scenes show the page behind them in the browser.** A scene with `Backdrop::Transparent` drew over solid black in the browser. wgpu lists only `Opaque` as a WebGPU canvas's alpha mode, though every browser canvas also takes `premultiplied`, so the canvas was set up opaque. Browser canvases now composite premultiplied, as native surfaces already did. A 3D view over a photo, a camera picture, or another 3D view shows them through its empty pixels. WebGL2, where wgpu refuses anything but opaque, is unchanged. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #715](https://github.com/leptoskit/leptoskit/pull/715)
