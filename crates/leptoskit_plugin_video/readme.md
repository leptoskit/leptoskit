<h1 align="center">leptoskit_plugin_video</h1>

<p align="center">
  <strong>leptoskit_plugin_video</strong> is the leptoskit plugin for host-owned URL video players with a <code>Video</code> surface.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_video"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_video"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_video"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_video.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_video"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_video/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Host-owned URL video players with an inline `Video` surface. Create a player from an HTTP(S) or file URL, then play, pause, seek, mute, loop, set rate, and snapshot status. Show the same player with `Video(player_id)`, which binds a host-owned id the way `Scene3d` binds a scene. Give the view a size and either an accessible label or `aria_hidden(true)`.

Add the crate as a direct dependency and `"video"` to `native.plugins`. Guests enable the `guest` feature and reuse `guest::Video`.

iOS and macOS use `AVPlayer` with an `AVPlayerLayer` surface. Android uses Media3 ExoPlayer with a `PlayerView`. The browser uses an HTML `<video>` element (Direct `web::Video`, or a `data-lk-video` surface). Windows, Linux, and HarmonyOS return `Unavailable`.

This MVP ports the core of [Expo Video](https://docs.expo.dev/versions/latest/sdk/video/): load a URL, transport, muted/loop/rate, and status. It does not yet cover text tracks, caching, AirPlay, fullscreen controls, or wiring iOS picture-in-picture enter to an attached player (see `picture-in-picture`). Audio-session / audio-focus configuration is left to later work.

## How it fits

<!-- {=p_videoSummary} -->

Add `leptoskit_plugin_video` as a direct dependency and select the `video` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Video` rather than resolving the plugin on every call. iOS and macOS use `AVPlayer` with an `AVPlayerLayer` surface, Android uses Media3 ExoPlayer with a `PlayerView`, and the browser uses an HTML `<video>` element; Windows, Linux, and HarmonyOS return `Unavailable`.

<!-- {/p_videoSummary} -->

## Use it

```rust,ignore
use leptoskit::prelude::*;
use leptoskit_plugin_video::{guest::Video, Source};

let video = Video::new()?;
let player = video.create(&Source::url("https://example.com/clip.mp4")?).await?;
video.play(player).await?;

View().child(
    Video(player.0)
        .class("w-full h-64")
        .aria_label("Product demo"),
);
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_video)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
