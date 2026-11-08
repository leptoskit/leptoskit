<h1 align="center">leptoskit_plugin_age_range</h1>

<p align="center">
  <strong>leptoskit_plugin_age_range</strong> asks the OS for a declared age band without inventing consent.
</p>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_age_range"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_age_range"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_age_range.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_age_range"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_age_range/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

## Summary

<!-- {=p_age_rangeSummary} -->

Add `leptoskit_plugin_age_range` as a direct dependency and select the `age-range` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::AgeRange` rather than resolving the plugin on every call. Query whether the host can ask, then request a declared age band around a threshold. Responses are Declined, Unavailable, or Shared with inclusive lower/upper bounds (`upper == 0` means open-ended). iOS uses Declared Age Range where the OS exposes it; Android, desktop, and the browser return `Unavailable` in this MVP. Never invents an age band when the OS declines or withholds one.

<!-- {/p_age_rangeSummary} -->
