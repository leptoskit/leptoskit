<h1 align="center">leptoskit_plugin_contacts</h1>

<p align="center">
  <strong>leptoskit_plugin_contacts</strong> is the leptoskit plugin for contacts permission, pick-one, and a bounded query.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_contacts"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_contacts"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_contacts"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_contacts.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_contacts"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_contacts/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Query permission without prompting, request access only after the user chooses, present the system contact picker for one bounded contact, or query at most 32 contacts with id, display name, and optional primary phone/email. This is not a CRM sync. Purpose strings and Android manifest permissions stay app-owned. Denied access never invents contacts. Desktop hosts and browsers report unavailable; the browser Contacts Picker API remains later scope.

## How it fits

<!-- {=p_contactsSummary} -->

Add `leptoskit_plugin_contacts` as a direct dependency and select the `contacts` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Contacts` rather than resolving the plugin on every call. Status never prompts; request permission only after the user chooses. Pick presents the system contact picker for one bounded contact. Query returns at most 32 contacts with id, display name, and optional primary phone/email. Purpose strings and manifest permissions stay app-owned; denied access never invents contacts. Browsers report unavailable; the Contacts Picker API remains later scope.

<!-- {/p_contactsSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_contacts::{Query, guest::Contacts};

let contacts = Contacts::new()?;
let status = contacts.status().await?;
let picked = contacts.pick().await?;
let page = contacts.query(Query::new(16)?).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_contacts)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
