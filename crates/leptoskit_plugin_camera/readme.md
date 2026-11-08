<h1 align="center">leptoskit_plugin_camera</h1>

<p align="center">
  <strong>leptoskit_plugin_camera</strong> is the leptoskit plugin for host-owned camera sessions with a <code>Camera</code> preview surface.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_camera"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_camera"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_camera"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_camera.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_camera"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_camera/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Host-owned camera sessions with an inline `Camera` preview. Create a session for the back or front lens, toggle the torch when the hardware supports it, take a bounded still into app cache, and snapshot status. Show the same session with `Camera(session_id)`, which binds a host-owned id the way `Scene3d` and `Video` bind theirs. Give the view a size and either an accessible label or `aria_hidden(true)`.

Ask for camera access through the `permissions` plugin. This plugin returns `PermissionDenied` or `Unavailable` when capture lacks a grant; it does not re-implement system permission prompts.

Add the crate as a direct dependency and `"camera"` to `native.plugins`. Guests enable the `guest` feature and reuse `guest::Camera`.

iOS and macOS use `AVCaptureSession` with an `AVCaptureVideoPreviewLayer` surface. Android uses CameraX with a `PreviewView`. The browser uses `getUserMedia` into an HTML `<video>` element (Direct `web::Camera`, or a `data-lk-camera` surface). Windows, Linux, and HarmonyOS return `Unavailable`.

This MVP ports the core of [Expo Camera](https://docs.expo.dev/versions/latest/sdk/camera/): session create/release, torch, still capture, status, and a preview surface. Video recording is not covered yet.

## How it fits

<!-- {=p_cameraSummary} -->

Add `leptoskit_plugin_camera` as a direct dependency and select the `camera` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Camera` rather than resolving the plugin on every call. Ask for camera access through the `permissions` plugin; this plugin returns `PermissionDenied` or `Unavailable` when capture lacks a grant. iOS and macOS use `AVCaptureSession` with an `AVCaptureVideoPreviewLayer` surface, Android uses CameraX with a `PreviewView`, and the browser uses `getUserMedia` into an HTML `<video>` element; Windows, Linux, and HarmonyOS return `Unavailable`. Video recording is not in this MVP.

<!-- {/p_cameraSummary} -->

## Use it

```rust,ignore
use leptoskit::prelude::*;
use leptoskit_plugin_camera::{guest::Camera, Facing};

let camera = Camera::new()?;
let session = camera.create(Facing::Back).await?;

View().child(
    Camera(session.0)
        .class("w-full h-64")
        .aria_label("Rear camera"),
);
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_camera)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
