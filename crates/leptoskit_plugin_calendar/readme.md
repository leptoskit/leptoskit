<h1 align="center">leptoskit_plugin_calendar</h1>

<p align="center">
  <strong>leptoskit_plugin_calendar</strong> is the leptoskit plugin for calendar permission, listing calendars, and creating or reading bounded events.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_calendar"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_calendar"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_calendar"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_calendar.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_calendar"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_calendar/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Query permission without prompting, request access only after the user chooses, list at most 64 calendars, create an event, get one event by id, or list at most 64 events in a bounded time window. This is not a full calendar sync and does not cover the legacy Expo Calendar API. Purpose strings and Android manifest permissions stay app-owned. Denied access never invents calendars or events. Desktop hosts and browsers report unavailable.

## How it fits

<!-- {=p_calendarSummary} -->

Add `leptoskit_plugin_calendar` as a direct dependency and select the `calendar` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Calendar` rather than resolving the plugin on every call. Status never prompts; request permission only after the user chooses. List calendars (max 64), create an event, get one event by id, or list events in a time window (max 64). Purpose strings and manifest permissions stay app-owned; denied access never invents calendars or events. Browsers and desktop hosts report unavailable; legacy Expo Calendar APIs remain out of scope.

<!-- {/p_calendarSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_calendar::{CreateEvent, ListEvents, guest::Calendar};

let calendar = Calendar::new()?;
let status = calendar.status().await?;
let calendars = calendar.list_calendars().await?;
let id = calendar
	.create_event(&CreateEvent::new(
		"primary",
		"Standup",
		1_700_000_000_000,
		1_700_000_360_000,
		Some("daily"),
		None::<String>,
	)?)
	.await?;
let event = calendar.get_event(&id).await?;
let page = calendar
	.list_events(ListEvents::new(1_700_000_000_000, 1_700_086_400_000, 16)?)
	.await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_calendar)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
