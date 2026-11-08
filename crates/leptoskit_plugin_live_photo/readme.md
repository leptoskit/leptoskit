<h1 align="center">leptoskit_plugin_live_photo</h1>

<p align="center">
  <strong>leptoskit_plugin_live_photo</strong> hosts Live Photo sessions and binds them to a <code>LivePhoto</code> surface.
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_live_photo"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_live_photo"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_live_photo.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_live_photo"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_live_photo/badge.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Create a session from a `ph://` Live Photo asset id, show it with `LivePhoto(session)`, then start hint/full playback or stop it. iOS/macOS use `PHLivePhotoView`. Android, browsers, Windows, Linux, and HarmonyOS return Unavailable. Editing and export remain later scope.

Enable the `guest` feature for native Wasm clients. Select `live-photo` in `native.plugins`.

<!-- {=p_live_photoSummary} -->

Add `leptoskit_plugin_live_photo` as a direct dependency and select the `live-photo` plugin in `native.plugins`. Guests enable `guest` and reuse `guest::LivePhoto`.

<!-- {/p_live_photoSummary} -->
