<h1 align="center">leptoskit_plugin_video_thumbnails</h1>

<p align="center">
  <strong>leptoskit_plugin_video_thumbnails</strong> extracts one JPEG frame from an app-owned video URI.
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_video_thumbnails"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_video_thumbnails"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_video_thumbnails.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_video_thumbnails"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_video_thumbnails/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Pass a video file URI, a time in milliseconds, and a JPEG quality (1–100). The host writes one temporary JPEG and returns its URI with width and height. iOS/macOS use `AVAssetImageGenerator`; Android uses `MediaMetadataRetriever`. Browsers, Windows, Linux, and HarmonyOS return Unavailable. Multi-frame extraction and custom sizes remain later scope.

Enable the `guest` feature for native Wasm clients. Select `video-thumbnails` in `native.plugins`.

<!-- {=p_video_thumbnailsSummary} -->

Add `leptoskit_plugin_video_thumbnails` as a direct dependency and select the `video-thumbnails` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::VideoThumbnails` rather than resolving the plugin on every call. Extract one JPEG frame from an app-owned video URI at a time offset. iOS and macOS use `AVAssetImageGenerator`; Android uses `MediaMetadataRetriever`; Windows, Linux, HarmonyOS, and the browser return `Unavailable`. Multi-frame extraction and custom sizes remain later scope.

<!-- {/p_video_thumbnailsSummary} -->
