<h1 align="center">leptoskit_plugin_background_task</h1>

<p align="center">
  <strong>leptoskit_plugin_background_task</strong> registers named OS background tasks with expiration-aware completion.
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_background_task"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_background_task"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_background_task.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_background_task"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_background_task/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Query availability, register or unregister a named task with a minimum interval hint (900s–86400s; WorkManager's periodic floor), and complete an in-flight OS invocation. This is **not** `leptoskit_plugin_background`: that plugin keeps an `AppService` awake with a foreground service / scheduled wake. Here the host only owns OS scheduler identifiers (`BGTaskScheduler` / `WorkManager`). Guaranteed cadence and running Wasm while suspended remain later scope.

Enable the `guest` feature for native Wasm clients. Select `background-task` in `native.plugins`.

<!-- {=p_background_taskSummary} -->

Add `leptoskit_plugin_background_task` as a direct dependency and select the `background-task` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::BackgroundTask` rather than resolving the plugin on every call. Register and unregister named OS background tasks, query availability, and complete an in-flight invocation before the OS expiration window. This is distinct from `leptoskit_plugin_background`, which keeps an `AppService` awake with a foreground service / scheduled wake. iOS uses `BGTaskScheduler`; Android uses `WorkManager`; Windows, Linux, HarmonyOS, and the browser return `Unavailable`. Guaranteed cadence and in-guest Wasm execution while suspended remain later scope.

<!-- {/p_background_taskSummary} -->
