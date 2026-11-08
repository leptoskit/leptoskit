<h1 align="center">leptoskit_plugin_sms</h1>

<p align="center">
  <strong>leptoskit_plugin_sms</strong> is the leptoskit plugin for owned-window SMS composer presentation.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_sms"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_sms"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_sms"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_sms.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_sms"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_sms/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Presents the system SMS composer with optional recipients and body. Completion means the OS finished its composer callback. It never confirms delivery or identifies a carrier outcome.

Enable the `guest` feature for native Wasm clients. Select `sms` in `native.plugins`. iOS uses `MFMessageComposeViewController`; Android uses `ACTION_SENDTO` (`smsto:`). Desktop and browser builds report `Unsupported`.

Outbound `sms:` URL opening remains `leptoskit_plugin_linking`. This crate does not absorb `sms:` scheme opens from linking.

| Platform        | Backend                         | Completion                             |
| --------------- | ------------------------------- | -------------------------------------- |
| iOS 17+         | MessageUI composer              | OS `didFinish` callback (not delivery) |
| Android 26+     | `ACTION_SENDTO` activity result | Activity result (not delivery)         |
| macOS / desktop | Unavailable metadata            | —                                      |
| Browser         | Explicitly unsupported          | `web::present` returns `Unavailable`   |

## Protocol

| Method       | ID | Request                           | Response                                             |
| ------------ | -- | --------------------------------- | ---------------------------------------------------- |
| Availability | 0  | empty                             | Unsupported=0, Absent=1, Ready=2, Inactive=3, Busy=4 |
| Present      | 1  | recipient count, recipients, body | empty                                                |

## How it fits

<!-- {=p_smsSummary} -->

Add `leptoskit_plugin_sms` as a direct dependency and select the `sms` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Sms` rather than resolving the plugin on every call. Completion means the OS finished its composer callback; it never confirms delivery. Outbound `sms:` URLs remain `leptoskit_plugin_linking`.

<!-- {/p_smsSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_sms::{Availability, Compose, guest::Sms};

let sms = Sms::new()?;
if sms.availability().await? == Availability::Ready {
    sms.present(&Compose::new(["5550100"], "Hello")?).await?;
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_sms)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
