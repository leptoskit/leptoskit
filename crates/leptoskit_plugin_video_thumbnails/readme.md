<h1 align="center">leptoskit_plugin_video_thumbnails</h1>

<p align="center">
  <strong>leptoskit_plugin_video_thumbnails</strong> extracts one JPEG frame from an app-owned video URI.
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_video_thumbnails"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_video_thumbnails"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_video_thumbnails.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_video_thumbnails"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_video_thumbnails/badge.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Pass a video file URI, a time in milliseconds, and a JPEG quality (1–100). The host writes one temporary JPEG and returns its URI with width and height. iOS/macOS use `AVAssetImageGenerator`; Android uses `MediaMetadataRetriever`. Browsers, Windows, Linux, and HarmonyOS return Unavailable. Multi-frame extraction and custom sizes remain later scope.

Enable the `guest` feature for native Wasm clients. Select `video-thumbnails` in `native.plugins`.

<!-- {=p_video_thumbnailsSummary} -->

Add `leptoskit_plugin_video_thumbnails` as a direct dependency and select the `video-thumbnails` plugin in `native.plugins`. Guests enable `guest` and reuse `guest::VideoThumbnails`.

<!-- {/p_video_thumbnailsSummary} -->
