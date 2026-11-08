<h1 align="center">leptoskit_plugin_mail_composer</h1>

<p align="center">
  <strong>leptoskit_plugin_mail_composer</strong> is the leptoskit plugin for owned-window mail composer presentation.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_mail_composer"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_mail_composer"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_mail_composer"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_mail_composer.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_mail_composer"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_mail_composer/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Presents the system mail composer with to, cc, bcc, subject and body. Completion means the OS finished its composer callback. It never confirms delivery. **Attachments are deferred in this MVP** and are not part of the wire protocol yet.

Enable the `guest` feature for native Wasm clients. Select `mail-composer` in `native.plugins`. iOS uses `MFMailComposeViewController`; Android uses `ACTION_SENDTO` (`mailto:`). Desktop and browser builds report `Unsupported`.

Outbound `mailto:` URL opening remains `leptoskit_plugin_linking`. This crate does not absorb `mailto:` scheme opens from linking.

| Platform        | Backend                         | Completion                             |
| --------------- | ------------------------------- | -------------------------------------- |
| iOS 17+         | MessageUI mail composer         | OS `didFinish` callback (not delivery) |
| Android 26+     | `ACTION_SENDTO` activity result | Activity result (not delivery)         |
| macOS / desktop | Unavailable metadata            | —                                      |
| Browser         | Explicitly unsupported          | `web::present` returns `Unavailable`   |

## Protocol

| Method       | ID | Request                            | Response                                             |
| ------------ | -- | ---------------------------------- | ---------------------------------------------------- |
| Availability | 0  | empty                              | Unsupported=0, Absent=1, Ready=2, Inactive=3, Busy=4 |
| Present      | 1  | to / cc / bcc lists, subject, body | empty                                                |

## How it fits

<!-- {=p_mail_composerSummary} -->

Add `leptoskit_plugin_mail_composer` as a direct dependency and select the `mail-composer` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::MailComposer` rather than resolving the plugin on every call. Completion means the OS finished its composer callback; it never confirms delivery. Attachments are deferred in this MVP. Outbound `mailto:` URLs remain `leptoskit_plugin_linking`.

<!-- {/p_mail_composerSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_mail_composer::{Availability, Compose, guest::MailComposer};

let mail = MailComposer::new()?;
if mail.availability().await? == Availability::Ready {
    mail.present(&Compose::new(
        ["friend@example.test"],
        None::<String>,
        None::<String>,
        "Hello",
        "Body text",
    )?).await?;
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_mail_composer)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
