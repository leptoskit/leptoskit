<h1 align="center">leptoskit_depth</h1>

<p align="center">
  <strong>leptoskit_depth</strong> reads metric depth from one camera picture, on the device, for AR on phones that measure none.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#the-model-file"><strong>The model file</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#speed"><strong>Speed</strong></a> ·
  <a href="https://docs.rs/leptoskit_depth"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_depth"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_depth"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_depth.svg"></a>
  <a href="https://docs.rs/leptoskit_depth"><img alt="docs.rs" src="https://docs.rs/leptoskit_depth/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

An AR game hides a creature behind the sofa by knowing how far away the sofa is. iPhones with LiDAR and Android phones with ARCore's Depth API measure that; many phones can't. The Solana Seeker has no ARCore at all, many ARCore phones lack the Depth API, and most iPhones have no LiDAR. On those, a neural network can look at the camera picture and say how far away each part of it is.

- **The model.** Depth Anything V2 Small, fine-tuned by its authors on indoor scenes to answer in metres, with its weights stored as 8-bit numbers: 27 MB, Apache-2.0. Only the Small size is under that licence.
- **Pure Rust.** It runs through [tract](https://github.com/sonos/tract) on the CPU, with no native libraries to ship: about a quarter of a second a picture on one core of an Apple M4.
- **Metres a room's size.** The model reads a small picture long by a steady factor that depends on the lens; the crate divides it out for the picture's focal length, measured against rooms of known depth.
- **Any camera layout.** RGB, RGBA, BGRA, or NV12 pixels, at any size, turned any quarter turn from upright. The depth comes back lying the way the picture did, so the camera's own display transform serves it.
- **Off the render thread.** `DepthWorker` reads at most one picture at a time, about once a second, on its own thread; the caller hands pictures over and reads the newest depth without ever waiting.

## How it fits

<!-- {=depthSummary} -->

It turns one camera picture into depth in metres, on the device, for AR on phones whose platform measures none: the Solana Seeker and other phones without ARCore, ARCore phones without its Depth API, and iPhones without LiDAR. The model is Depth Anything V2 Small, fine-tuned for rooms, and it runs in pure Rust on the CPU in about a quarter of a second.

<!-- {/depthSummary} -->

It is a host library: the Wasm guest never sees camera pictures, so the depth is read in the native app, where the camera is, and answers the same hit tests and occlusion that platform depth does. `leptoskit_xr` is the intended caller: for a session that asks for depth on a phone whose platform has none, it hands camera frames to a `DepthWorker` and serves the newest `DepthMap` as the frame's depth picture.

## The model file

The 27 MB model is not in this crate or its repository. `MODEL` pins it by SHA-256 and says where to fetch it; `MODEL_LICENSE` is its licence, which ships with it.

```sh
mkdir -p assets/depth
curl -L -o assets/depth/depth_anything_v2_metric_indoor_small.onnx \
  https://github.com/pina-rs/reapers/releases/download/depth-model-v1/depth_anything_v2_metric_indoor_small.onnx
echo "b81c4fae8985bd73d5026ffc1ebf545ff01d73fbf4d534a06c811abfcadb6dbd  assets/depth/depth_anything_v2_metric_indoor_small.onnx" | shasum -a 256 -c
```

Put it in the app's `assets/` folder with its licence: `leptoskit build` bundles that folder into the iOS app and packs it into the Android APK, and the host reads its bytes through the same asset source 3D scenes load from. `MODEL.verify(&bytes)` checks it before `DepthModel::from_bytes` loads it.

The file was converted for the Reapers game from the authors' weights (`depth-anything/Depth-Anything-V2-Metric-Indoor-Small-hf` at revision `8078d68a`): traced for a 252 × 448 upright picture, exported to ONNX, and quantised dynamically to unsigned 8-bit weights with ONNX Runtime's tools. Quantising changed the answers by 1 to 2% and the accuracy against NYU Depth v2 not at all.

## Use it

```toml
[dependencies]
leptoskit_depth = "0.3"
```

```rust
use leptoskit_depth::{DepthModel, DepthWorker, Frame, Orientation, Picture, Pixels, Plane, MODEL};

/// What the game keeps with each picture: where the camera was.
type CameraPose = [f32; 7];

fn start(bytes: &[u8]) -> Result<DepthWorker<CameraPose>, Box<dyn std::error::Error>> {
	MODEL.verify(bytes)?;
	let model = DepthModel::from_bytes(bytes)?;
	Ok(DepthWorker::spawn(model)?)
}

fn each_camera_frame(worker: &DepthWorker<CameraPose>, rgb: Plane, time: f64, pose: CameraPose) {
	// Cheap: build the picture only when the worker would take it.
	if worker.ready() {
		let picture = Picture::new(Pixels::Rgb8(rgb)).with_orientation(Orientation::Clockwise);
		worker.submit(Frame { picture, time, context: pose });
	}

	if let Some(estimate) = worker.latest() {
		// Metres straight ahead, as seen from `estimate.context`, the pose
		// the picture was taken from, `time - estimate.time` seconds ago.
		let ahead = estimate.depth.sample(0.5, 0.5);
		println!("{ahead:?} m");
	}
}
```

A `DepthMap` holds metres along the camera's forward axis (not along each ray), in rows over the picture as the camera read it, zero where the model did not look. `sample(u, v)`, `point(u, v)` (camera space, +z forward), and `distance(u, v)` (along the ray) read it; `to_le_bytes()` is the `Meters32` layout `leptoskit_xr`'s `DepthImage` takes.

### What it reads, and what it can't

- The model takes a 9:16 upright picture. A picture of another shape is cropped from its middle, and the depth outside the crop is unknown: a 4:3 phone picture keeps three quarters of its width, and a landscape picture only a narrow middle strip.
- Give the picture's intrinsics. Without them the lens is assumed to see 68° across the longer side, a typical phone's main camera, and the metres can be a fifth out.
- Against nine rooms of known depth the corrected metres read between 13% short and 14% long. A whole picture reads long or short together, so nearer and farther stay in order: measure against something known, such as the floor, where that matters.
- One picture at a time: the depth is the room as it was when the picture was taken, so keep where the camera was then (the `Frame`'s context) and cast rays from there.
- It does not see in the dark, through glass, or into mirrors.

## Speed

On an Apple M4 (10 cores), the pinned model on a 252 × 448 picture, warm medians of 7 runs, measured while the machine was lightly loaded (load average about 5). With other builds running (load average 15 to 20) every runtime read 450 ms to 1 s and their order was not stable, so compare numbers taken together:

| Runtime                                                 | 1 thread | 2 threads | 4 threads | Load  | Output                     |
| ------------------------------------------------------- | -------- | --------- | --------- | ----- | -------------------------- |
| tract 0.23.9 (this crate)                               | 264 ms   | 258 ms    | 247 ms    | 0.5 s | correct                    |
| ONNX Runtime (`ort` 2.0.0-rc.15), no graph optimisation | 252 ms   | 191 ms    | 170 ms    | 3.7 s | correct, matches tract     |
| ONNX Runtime, its default optimisation                  | 227 ms   | 165 ms    | 148 ms    | 3.7 s | wrong: a near-constant map |

tract is within 1.05× of ONNX Runtime on one thread and 1.35× on two, needs no native library per platform, and reads the quantised model correctly. ONNX Runtime's graph optimisations broke this model on Apple silicon outright. The worker runs the model on one thread, leaving the rest of the phone to draw. The Reapers game measured ONNX Runtime at about 0.5 s a picture on the Seeker's two cores; tract has not yet been timed on a phone.

```sh
LEPTOSKIT_DEPTH_MODEL=assets/depth/depth_anything_v2_metric_indoor_small.onnx \
  cargo run --release -p leptoskit_depth --example benchmark
```

The tests in `tests/model.rs` run the model on three photographs of real rooms when `LEPTOSKIT_DEPTH_MODEL` names the file, and pass with a note when it doesn't.

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_depth)
- [Augmented reality on phones](https://leptoskit.dev/docs/guide/34-augmented-reality.html)
- [Depth Anything V2](https://github.com/DepthAnything/Depth-Anything-V2)
- [Repository](https://github.com/leptoskit/leptoskit)
