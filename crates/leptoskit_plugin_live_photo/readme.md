<h1 align="center">leptoskit_plugin_live_photo</h1>

<p align="center">
  <strong>leptoskit_plugin_live_photo</strong> hosts Live Photo sessions and binds them to a <code>LivePhoto</code> surface.
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_live_photo"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_live_photo"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_live_photo.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_live_photo"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_live_photo/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Create a session from a `ph://` Live Photo asset id, show it with `LivePhoto(session)`, then start hint/full playback or stop it. iOS/macOS use `PHLivePhotoView`. Android, browsers, Windows, Linux, and HarmonyOS return Unavailable. Editing and export remain later scope.

Enable the `guest` feature for native Wasm clients. Select `live-photo` in `native.plugins`.

<!-- {=p_live_photoSummary} -->

Add `leptoskit_plugin_live_photo` as a direct dependency and select the `live-photo` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::LivePhoto` rather than resolving the plugin on every call. Create a session from an app-owned Live Photo asset URI, bind it with `LivePhoto(session)`, then start or stop playback. iOS and macOS use `PHLivePhotoView`; Android, Windows, Linux, HarmonyOS, and the browser return `Unavailable`. Editing and export remain later scope.

<!-- {/p_live_photoSummary} -->
