<h1 align="center">leptoskit_plugin_asset</h1>

<p align="center">
  <strong>leptoskit_plugin_asset</strong> resolves packaged asset names to local URIs without inventing missing files.
</p>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_asset"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_asset"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_asset.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_asset"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_asset/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

## Summary

<!-- {=p_assetSummary} -->

Add `leptoskit_plugin_asset` as a direct dependency and select the `asset` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Asset` rather than resolving the plugin on every call. Query whether packaged-asset resolution is available, then resolve a logical asset name to a local URI. This MVP returns Unavailable on every host until packaged-asset ownership wiring lands. Never invents a file URI for a missing asset.

<!-- {/p_assetSummary} -->
