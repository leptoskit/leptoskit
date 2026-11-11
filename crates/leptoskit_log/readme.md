<h1 align="center">leptoskit_log</h1>

<p align="center">
  <strong>leptoskit_log</strong> sends leptoskit's own records to the platform's log, with a panic hook, so a failure on a device says what happened.
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_log"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_log"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_log.svg"></a>
  <a href="https://docs.rs/leptoskit_log"><img alt="docs.rs" src="https://docs.rs/leptoskit_log/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

<!-- {=logSummary} -->

It sends leptoskit's own records to the platform's log: logcat on Android, the unified log (`os_log`) on iOS and macOS, standard error on Linux and Windows, and the console in a browser. The runtime installs it when an app starts, with a panic hook, so a failure on a device says which plugin, call, node, and payload it was about. Nothing leaves the device.

<!-- {/logSummary} -->

Apps don't call it: the native runtime installs it when it creates an app, and the web renderer when it first draws. Read the records with the platform's own tools:

| Platform          | Where records go                           | How to read them                                                    |
| ----------------- | ------------------------------------------ | ------------------------------------------------------------------- |
| Android           | logcat, tagged `leptoskit/<part>`          | `adb logcat \| grep leptoskit/`                                     |
| iOS and macOS     | the unified log, subsystem `dev.leptoskit` | `log stream --predicate 'subsystem == "dev.leptoskit"'`, or Console |
| Linux and Windows | standard error                             | the terminal that started the app                                   |
| Web               | the browser console                        | the browser's developer tools                                       |

Release builds keep warnings and errors; debug builds add `info`. Set another level in `leptoskit.toml`:

```toml
[native]
log-level = "debug"
```

Nothing is sent anywhere: there is no telemetry and no crash-reporting service. A crash reporter an app adds of its own sees the same records through the `log` facade.
