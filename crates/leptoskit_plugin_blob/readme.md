<h1 align="center">leptoskit_plugin_blob</h1>
<p align="center"><strong>leptoskit_plugin_blob</strong> owns bounded byte blobs so guests avoid copying large payloads on every call.</p>
<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_blob"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_blob"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_blob.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_blob"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_blob/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

## Summary

<!-- {=p_blobSummary} -->

Add `leptoskit_plugin_blob` as a direct dependency and select the `blob` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Blob` rather than resolving the plugin on every call. Query availability, create an owned byte blob, read a bounded slice, and release it without copying the whole payload through every control message. This MVP keeps blobs in process memory on Apple, Android, Linux, Windows, and the browser; durable shared-memory / object-URL backends remain later scope.

<!-- {/p_blobSummary} -->
