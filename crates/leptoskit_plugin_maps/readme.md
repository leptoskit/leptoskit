<h1 align="center">leptoskit_plugin_maps</h1>

<p align="center">
  <strong>leptoskit_plugin_maps</strong> hosts map sessions and binds them to a <code>Map</code> surface.
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_maps"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_maps"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_maps.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_maps"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_maps/badge.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Create a map session, show it with `Map(session)`, then set the camera and up to 32 markers. iOS/macOS use MapKit; Android uses Google Maps (`MapView`, app-supplied API key). Browsers, Windows, Linux, and HarmonyOS return Unavailable. User location, polygons, and clustering remain later scope.

Enable the `guest` feature for native Wasm clients. Select `maps` in `native.plugins`.

<!-- {=p_mapsSummary} -->

Add `leptoskit_plugin_maps` as a direct dependency and select the `maps` plugin in `native.plugins`. Guests enable `guest` and reuse `guest::Maps`.

<!-- {/p_mapsSummary} -->
