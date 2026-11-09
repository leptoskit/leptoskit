<h1 align="center">leptoskit_plugin_diagnostics</h1>
<p align="center"><strong>leptoskit_plugin_diagnostics</strong> records named spans and events for startup and interaction diagnostics.</p>
<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_diagnostics"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_diagnostics"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_diagnostics.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_diagnostics"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_diagnostics/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

## Summary

<!-- {=p_diagnosticsSummary} -->

Add `leptoskit_plugin_diagnostics` as a direct dependency and select the `diagnostics` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Diagnostics` rather than resolving the plugin on every call. Query availability, start and end named spans, and record one-shot events for startup and interaction timing. This MVP is an in-process no-op allocator on Apple, Android, Linux, Windows, and the browser — spans succeed without choosing a hosted exporter; OpenTelemetry / OS logging backends remain later scope.

<!-- {/p_diagnosticsSummary} -->
