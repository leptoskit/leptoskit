<h1 align="center">leptoskit_xr</h1>

<p align="center">
  <strong>leptoskit_xr</strong> is augmented and virtual reality for leptoskit: phones through ARKit and ARCore, headsets through OpenXR, and the browser through WebXR.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_xr"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_xr"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_xr"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_xr.svg"></a>
  <a href="https://docs.rs/leptoskit_xr"><img alt="docs.rs" src="https://docs.rs/leptoskit_xr/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

A game that hides creatures behind your sofa needs to know where the floor is, how far away the sofa is, and where the phone is pointing, sixty times a second. This crate gets that from the platform and hands the app a stream of frames.

- **Modes.** `Orientation` (look around by turning, from the rotation sensor, on any phone), `PhoneAr` (the camera picture with 6DoF tracking), `ImmersiveVr` and `ImmersiveAr` (a headset, the second showing the room through passthrough).
- **Frames.** Each frame carries the viewer's pose, one view per eye or screen with its projection, detected planes with their outlines and labels, anchors, a light estimate, and controllers, hands, touches, or gaze. Planes are sent only when they change.
- **The real world.** Hit tests cast a ray (or a screen tap) against planes, the depth picture, and the platform's own estimates. Anchors stay pinned as the platform corrects its map.
- **Backends.** ARKit on iOS (LiDAR scene depth where the device has it), ARCore on Android (Depth API, ambient light), OpenXR on Meta Quest, Android XR, and PC headsets (the `openxr` feature), WebXR in the browser (the `web` feature), Core Motion and the Android rotation vector for orientation, and a deterministic replay of recorded frames.

Tracking and rendering stay in native code. A Wasm guest receives poses and planes, never the camera or depth pictures; the host's renderer draws the scene over the camera image or into the headset and hides what is behind real surfaces.

Coordinates are right-handed metres with +Y up, and a viewer looks down its own −Z axis, as every platform underneath reports them.

## How it fits

<!-- {=xrSummary} -->

It gives an app the real world to play in: camera passthrough, the device's position and rotation, floors and walls, depth for hiding things behind real furniture, and hit tests, through ARKit on iOS, ARCore on Android, OpenXR on Meta Quest and Android XR headsets, and WebXR in the browser. Phones without AR fall back to looking around by turning, and a recording plays back the same frames for tests and simulators.

<!-- {/xrSummary} -->

Add the crate as a direct dependency of the app and select `"xr"` in `native.plugins`; the generated iOS, Android, and Quest hosts register it. It is an async plugin, so `[native.plugin-options.xr]` configures the host: `max-frame-rate` caps how many frames a second the guest receives, and `replay` plays a recording instead of the device, for simulators. Guests enable `guest` and use `leptoskit_xr::Xr`; the browser build enables `web` and uses the same type over WebXR.

| Feature      | What it adds                                                                                                   |
| ------------ | -------------------------------------------------------------------------------------------------------------- |
| _(none)_     | The types, the binary protocol, and software hit tests. Builds anywhere.                                       |
| `guest`      | `guest::Xr`, the client for interpreter guests.                                                                |
| `host`       | `host::XrRuntime`, the backends, recordings, and the `Backend` trait for your own.                             |
| `async-host` | The plugin generated hosts register (the CLI turns it on).                                                     |
| `openxr`     | The OpenXR backend. A renderer supplies each session's graphics device through `OpenXrBackend::with_renderer`. |
| `web`        | `web::Xr` over WebXR, with a `deviceorientation` fallback.                                                     |

## Use it

```toml
[dependencies]
leptoskit_xr = { version = "0.3", features = ["guest"] }

# leptoskit.toml
[native]
plugins = ["xr", "permissions"]

[native.plugin-options.xr]
max-frame-rate = 30
```

```rust
use leptoskit_xr::{Features, HitTestRequest, Mode, SessionConfig, Xr};

async fn hunt() -> Result<(), leptoskit_xr::Error> {
	let xr = Xr::new()?;
	let mode = xr
		.capabilities()
		.await?
		.first_of(&[Mode::PhoneAr, Mode::Orientation])
		.ok_or(leptoskit_xr::Error::Unavailable)?;
	let mut session = xr
		.start(SessionConfig::new(mode).prefer(Features::PLANES | Features::DEPTH | Features::HIT_TEST))
		.await?;

	loop {
		let frame = session.next_frame().await?;
		// Place things relative to `frame.viewer` and `frame.planes`; ask
		// `session.hit_test(HitTestRequest::screen(x, y))` where the user taps.
		let _ = (frame, HitTestRequest::screen(0.5, 0.5));
	}
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_xr)
- [Augmented reality on phones](https://leptoskit.dev/docs/guide/34-augmented-reality.html)
- [VR and AR on headsets](https://leptoskit.dev/docs/guide/35-headsets.html)
- [Meta Quest](https://leptoskit.dev/docs/platforms/quest.html)
- [Repository](https://github.com/leptoskit/leptoskit)
