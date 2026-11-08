---
"leptoskit_depth": minor
---

# Depth from the camera picture, for phones that measure none

The new `leptoskit_depth` crate reads metric depth from one camera picture on the device, for AR on phones whose platform has no depth: the Solana Seeker and other phones without ARCore, ARCore phones without the Depth API, and iPhones without LiDAR. It runs Depth Anything V2 Small, the authors' fine-tune for rooms, quantised to 27 MB, in pure Rust through tract: about a quarter of a second a picture on one core of an Apple M4, with no native library to ship.

`DepthModel::from_bytes` loads the model and `estimate` reads a `Picture` (RGB, RGBA, BGRA, or NV12 pixels, any quarter turn from upright, with the lens's intrinsics) into a `DepthMap` of metres along the camera's forward axis, laid over the picture as the camera read it, its scale corrected for the lens. `DepthWorker` runs it on its own thread about once a second for an AR runtime that hands over frames and must never wait. The model file is not in the crate: `MODEL` pins it by SHA-256 and says where to fetch it, `MODEL_LICENSE` is its Apache-2.0 licence, and an app ships both in its assets.

The `xr` plugin does not call it yet, and it has not run on a phone.
