<h1 align="center">leptoskit_plugin_web_browser</h1>

<p align="center">
  <strong>leptoskit_plugin_web_browser</strong> is the leptoskit plugin for dismissible in-app HTTP(S) browsing.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_web_browser"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_web_browser"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_web_browser"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_web_browser.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_web_browser"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_web_browser/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Opens an absolute HTTP(S) URL in an in-app / system browser sheet and answers when that sheet is dismissed. It is **not** `web-auth`: there is no OAuth return address, private-use scheme, or Auth Tab redirect. It is **not** `linking.open`: linking opens outbound destinations without waiting for dismiss.

Enable the `guest` feature for native Wasm clients. Select `web-browser` in `native.plugins`. iOS uses `SFSafariViewController`; Android uses Chrome Custom Tabs. Desktop metadata marks the plugin unavailable (use `linking` for fire-and-forget opens). Browser builds report `Unsupported`.

| Platform        | Backend                  | Completion              |
| --------------- | ------------------------ | ----------------------- |
| iOS 17+         | `SFSafariViewController` | Dismiss callback        |
| Android 26+     | Chrome Custom Tabs       | Activity result         |
| macOS / desktop | Unavailable metadata     | —                       |
| Browser         | Explicitly unsupported   | `web::open` Unavailable |

## Protocol

| Method       | ID | Request             | Response                                   |
| ------------ | -- | ------------------- | ------------------------------------------ |
| Availability | 0  | empty               | Unsupported=0, Ready=1, Inactive=2, Busy=3 |
| Open         | 1  | length-prefixed URL | empty                                      |

## How it fits

<!-- {=p_web_browserSummary} -->

Add `leptoskit_plugin_web_browser` as a direct dependency and select the `web-browser` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::WebBrowser` rather than resolving the plugin on every call. This opens HTTP(S) in an in-app or system browser and answers when it is dismissed. It is not `web-auth` (no OAuth return address) and not `linking.open` (which does not wait for dismiss).

<!-- {/p_web_browserSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_web_browser::{Availability, Open, guest::WebBrowser};

let browser = WebBrowser::new()?;
if browser.availability().await? == Availability::Ready {
    browser.open(&Open::url("https://example.test/")?).await?;
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_web_browser)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
