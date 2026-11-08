<h1 align="center">leptoskit_plugin_media_library</h1>

<p align="center">
  <strong>leptoskit_plugin_media_library</strong> lists and saves system photo-library assets with hard bounds.
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_media_library"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_media_library"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_media_library.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_media_library"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_media_library/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Queries library authorization without prompting, lists a bounded set of image/video assets, and saves an app-owned file into the system library. Denied access never invents rows. Change observation and albums are out of this MVP.

Enable the `guest` feature for native Wasm clients. Select `media-library` in `native.plugins`.

<!-- {=p_media_librarySummary} -->

Add `leptoskit_plugin_media_library` as a direct dependency and select the `media-library` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::MediaLibrary`. Status never prompts. List returns at most 64 assets with id, media kind, URI, dimensions, and creation time. Save copies an app-owned file URI into the system library. Change observation, albums, and limited-library nuance remain later scope. Browsers report Unavailable.

<!-- {/p_media_librarySummary} -->
