<h1 align="center">leptoskit_plugin_app_integrity</h1>

<p align="center">
  <strong>leptoskit_plugin_app_integrity</strong> requests opaque platform attestation tokens for server verification.
</p>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_app_integrity"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_app_integrity"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_app_integrity.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_app_integrity"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_app_integrity/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

## Summary

<!-- {=p_app_integritySummary} -->

Add `leptoskit_plugin_app_integrity` as a direct dependency and select the `app-integrity` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::AppIntegrity` rather than resolving the plugin on every call. Query whether attestation is available, then request a bounded token for an app-owned challenge. Tokens are opaque bytes for server verification; this MVP returns Unavailable on every host until App Attest / Play Integrity wiring lands. Never invents a passing attestation.

<!-- {/p_app_integritySummary} -->
