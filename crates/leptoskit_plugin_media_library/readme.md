<h1 align="center">leptoskit_plugin_media_library</h1>

<p align="center">
  <strong>leptoskit_plugin_media_library</strong> lists and saves system photo-library assets with hard bounds.
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_media_library"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_media_library"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_media_library.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_media_library"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_media_library/badge.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Queries library authorization without prompting, lists a bounded set of image/video assets, and saves an app-owned file into the system library. Denied access never invents rows. Change observation and albums are out of this MVP.

Enable the `guest` feature for native Wasm clients. Select `media-library` in `native.plugins`.

<!-- {=p_media_librarySummary} -->

Add `leptoskit_plugin_media_library` as a direct dependency and select the `media-library` plugin in `native.plugins`. Guests enable `guest` and reuse `guest::MediaLibrary`.

<!-- {/p_media_librarySummary} -->
