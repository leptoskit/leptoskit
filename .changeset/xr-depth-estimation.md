---
"leptoskit_xr": minor
"leptoskit_cli": minor
---

# Estimated depth for phones that measure none

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
