---
"leptoskit_xr": minor
"leptoskit_3d": minor
"leptoskit_bridge_ffi": minor
"leptoskit_cli": minor
---

# 3D scenes over the live camera on iOS and Android

A `Scene3d` whose backdrop is `Backdrop::Camera` now draws over the camera while an `xr` session runs on a phone. The generated iOS and Android apps hand the scene the session's newest frame: the camera picture behind the models, turned upright for the screen's rotation and cropped to fill the view, the session's own viewpoint in place of the scene's camera, and the room's measured depth hiding the parts of the scene behind real things. Start the session with `Features::CAMERA_IMAGE` (and `Features::DEPTH` for occlusion). Without a session the scene draws as before, transparent behind its models.

Android phones without ARCore, such as the Solana Seeker, now show the back camera in `Orientation` sessions too, through Camera2, with the rotation sensor turning the view and the camera's own field of view keeping the scene to the picture's scale. Such a session leaves the picture out, rather than failing, until the user allows the camera, unless it requires `CAMERA_IMAGE`.

On Android a `Scene3d` showing the camera moves below the window's own views, so the interface laid over it stays visible.

For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::set_camera_feed` takes the picture a camera backdrop draws over, `View::set_rotation` the interface's rotation, and the bridge adds `lk_scene3d_view_set_rotation` and `lk_scene3d_view_shows_camera`. `leptoskit_xr`'s new `scene3d` feature makes the plugin's sessions that feed, and `leptoskit build` turns it on, with any other async plugin's `scene3d` feature, when the app shows 3D scenes. `leptoskit_xr::host::install_depth_estimator` (or `XrRuntime::set_depth_estimator`) takes a `DepthEstimator`, which turns small CPU copies of the camera picture (`CameraFrame`, with the camera's `Intrinsics`) into depth for phones that measure none.
