<h1 align="center">leptoskit_3d</h1>

<p align="center">
  <a href="https://leptoskit.dev/"><img src="https://raw.githubusercontent.com/leptoskit/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_3d</strong> draws host-owned 3D scenes of glTF models, with skins, morph targets, and animation, on the GPU with wgpu.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_3d"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_3d"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_3d"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_3d.svg"></a>
  <a href="https://docs.rs/leptoskit_3d"><img alt="docs.rs" src="https://docs.rs/leptoskit_3d/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Some apps show models that move: a character that walks and waves, a product turning on a stand, creatures standing in a room seen through the phone's camera. `leptoskit_3d` keeps such a scene on the host, animates it on the display's own clock, and draws it on the GPU, so the app's interface code only says what should happen.

- **Models:** glTF 2.0 (`.glb`, or `.gltf` with its files), including `EXT_meshopt_compression`, `KHR_mesh_quantization` and `KHR_texture_transform` (as gltfpack writes them), decoded in pure Rust. Skins, morph targets by name, node names, and animation of transforms and weights (step, linear, cubic spline). Metallic-roughness, unlit, and emissive-strength materials with opaque, masked, and blended alpha. PNG and JPEG textures.
- **Scenes:** instances and groups, named parts shown or hidden, clips played, crossfaded, and stopped, named parts moved by the app on top of their clips, morph weights, material overrides (tint, emissive, opacity, a Fresnel rim), additive blending for glows, nodes attached to named parts, a perspective camera or explicit matrices, one directional light, and raycasts that report the part, distance, point, and normal.
- **Drawing:** a forward renderer with 4× MSAA, HDR lighting tonemapped with Khronos PBR Neutral, a prefiltered environment baked once per device, a soft shadow under each model, GPU skinning and morphs, instancing, culling, and transparency sorted back to front.
- **Targets:** any wgpu texture, with one view or several (stereo by viewports or by layers), over a clear colour or a camera image (Y and CbCr planes or RGBA, with a display transform), with real-world depth to hide what is behind it.

## How it fits

<!-- {=scene3dSummary} -->

It draws 3D scenes the host owns: glTF models with skins, morph targets, and animation, lit and shaded on the GPU with wgpu, in a `Scene3d` view. A guest describes a scene with compact ops and hears back through events; the host animates and draws it on the display link, never in the guest's frame loop.

<!-- {/scene3dSummary} -->

Without the default `renderer` feature, the crate is only the ops, events, and limits, and builds for any target, Wasm guests included. `guest` adds the client a native app's Wasm guest sends ops with, through the `leptoskit.scene3d` plugin. `host` adds that plugin and the hub native hosts show scenes with; `leptoskit_bridge_ffi`'s `scene3d` feature hosts them in a `CAMetalLayer` on iOS and macOS and a transparent `TextureView` on Android. Android scenes take their place in the normal view hierarchy, so backgrounds stay behind their models and a HUD stays above them, including over the camera. `web` draws scenes into the page's `Scene3d` canvases with WebGPU, or WebGL2 where the browser has no WebGPU.

Every op is bounded before it applies, and every model is decoded within limits, so a scene from untrusted code cannot make the host allocate or loop without limit.

## Use it

In an app, show a scene with a `Scene3d` view and build it with `client::Scene` (through `leptoskit`'s `3d` feature). To draw a scene yourself, such as into an OpenXR swapchain:

```rust
use std::sync::Arc;

use leptoskit_3d::*;

let gpu = Gpu::headless()?;
let mut renderer = Renderer::new(&gpu);
let world = renderer.world_mut();
world.apply([
	Op::LoadAsset { asset: AssetId(1), path: "models/character.glb".into() },
	Op::Spawn { node: NodeId(1), asset: Some(AssetId(1)), parent: None },
	Op::Play { node: NodeId(1), play: Play::new("spin") },
]);
for (id, path) in world.take_loads() {
	let model = Asset::from_slice(&std::fs::read(format!("assets/{path}"))?)?;
	world.asset_loaded(id, Ok(Arc::new(model)));
}
renderer.advance(1.0 / 60.0);
// Offscreen: RGBA bytes with premultiplied alpha.
let pixels = renderer.render_to_rgba(512, 512)?;
```

To draw into your own texture each frame, pass a `RenderTarget` with its view, format, and size, and optionally your own `ViewParams` per eye, a `Background`, and `Occlusion`.

## Learn more

- [3D scenes guide](https://leptoskit.dev/docs/guide/33-3d-scenes.html)
- [Crate documentation](https://docs.rs/leptoskit_3d)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
