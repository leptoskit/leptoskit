# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

#### Estimated depth for phones that measure none

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #704](https://github.com/leptoskit/leptoskit/pull/704)

The `xr` plugin now gives depth to phones whose platform has none: the Solana Seeker and other phones without ARCore, ARCore phones without the Depth API, and iPhones without LiDAR. Put the `leptoskit_depth` model in the app's `assets/` folder and name it:

```toml
[native.plugin-options.xr]
depth-model = "depth/depth_anything_v2_metric_indoor_small.onnx"
```

The native app loads the model in the background when it starts and checks it against `leptoskit_depth::MODEL`; a missing or different file is logged and leaves depth off. Once it has loaded, sessions on those phones offer `Features::DEPTH`: the model reads about one camera picture a second on its own thread, and the newest reading becomes the frame's depth, so a `Scene3d` over the camera hides things behind real furniture and `DEPTH` hit tests reach the room. Phones that measure depth keep their own.

A reading can be a second old, so hit tests cast against it from where the phone was when the picture was taken (`XrFrame::depth_view`), and occlusion turns it to the current view (`XrFrame::depth_transform`, built on `hit_test::view_to_view`). The new `depth-estimation` feature adds `host::ModelEstimator` and `host::install_depth_model`, for hosts that load the model themselves; `leptoskit build` turns it on only for apps that set `depth-model`.

`DepthEstimator::latest` now returns a `DepthEstimate`, the depth with the view and time of the frame it was read from, and `CameraFrame` carries that frame's `view` in place of `pose`.

Plugin crates can declare `option-features` in their runtime metadata, such as `{ depth-model = "depth-estimation" }`: `leptoskit build` turns the feature on when the app sets the option.

Nothing here has run on a phone yet.

#### Distinguish measured and estimated depth

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)

`Frame::depth` now carries `Option<DepthSource>`: `None`, `Measured`, or `Estimated`. The host tags depth from the camera model separately from platform depth, including on phones with AR tracking and planes but no Depth API or LiDAR. Model estimates can have a different scale from the tracked floor: find each picture's own floor before scaling it to the measured floor's height.

Depth hit tests report `HitKind::EstimatedDepth` for model estimates and `HitKind::Depth` for measured depth, using the source of the picture the request actually hit. Recording and playback retain the source; existing version-one recordings still load, with depth treated conservatively as estimated because they did not record its source.

Apps that checked `frame.depth` as a boolean now use `frame.depth.is_some()` or match its source. Rebuild native hosts alongside their guests.

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

#### Augmented and virtual reality, and Meta Quest apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #659](https://github.com/leptoskit/leptoskit/pull/659)

The new `leptoskit_xr` crate and its `xr` plugin put an app in the real world. Start a session in the best mode the device has — `PhoneAr` through ARKit on iOS and ARCore on Android, `ImmersiveVr` and `ImmersiveAr` on headsets through OpenXR, WebXR in the browser, or `Orientation`, which turns the view with the phone on any device — and read a stream of frames: where the viewer is, each view's projection, the floors, tables, and walls found so far, pinned anchors, the room's light, and controllers and hands. Hit tests find where a tap or a controller's ray meets the real world, against planes and, on phones that measure it, the depth of the room. The camera and depth pictures stay in the native app for the renderer, and the frames your code receives are capped at `[native.plugin-options.xr] max-frame-rate`, always the newest. A recording replays a session exactly, for tests and simulators, with `replay = "…"`.

`leptoskit build --platform quest` builds the Android project as a Meta Quest app, with the manifest Horizon OS expects: OpenXR, head tracking, optional hand tracking and passthrough, the supported headsets, and a full-screen immersive launcher. `[native.quest]` configures it, and `leptoskit dev --platform quest` installs onto a connected headset.

With the `3d` feature, `host::scene3d::draw` draws a `leptoskit_3d` scene into a session: the camera picture behind it, real-world depth hiding what is behind real things, and one view per eye on a headset.

The backends build for their platforms but have not yet run on a device, so treat them as experimental.

### Fixes

#### Plugin options the app would crash on stop the build instead

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #729](https://github.com/leptoskit/leptoskit/pull/729)

An Android app whose `leptoskit.toml` had an empty `[native.plugin-options.websocket]` built, installed, and then stopped at launch with `NativeApplicationInitializationFailure: register plugin: InvalidArgument`. The WebSocket plugin can't start without `origins`, the build only checked that the table was there, and the error didn't say which plugin had failed.

`leptoskit build` now stops with the plugin and the option it needs:

```
plugin `websocket` needs `origins` in [native.plugin-options.websocket]: without it, the app stops at launch when the plugin starts
```

An async plugin lists the options its factory can't start without in its runtime metadata, and an empty string, list, or table counts as left out:

```toml
[package.metadata.leptoskit.runtime]
name = "websocket"
rust-factory = "from_config"
rust-async = true
required-options = ["origins"]
```

`http` and `websocket` require `origins`, `shell` requires `commands`, and `xr` requires nothing. Each crate's tests hold its factory to its list.

When a plugin still fails to start, the error names it: ``register plugin `websocket`: InvalidArgument``. Generated registrants install every plugin through the new `App::register`, and a host directory that can't be used is reported as `app context: …` rather than as a plugin's failure.

The `shell` plugin is now left out of iOS, Android, and HarmonyOS builds, where it can't start processes and its factory always failed, which also stopped the app at launch.
