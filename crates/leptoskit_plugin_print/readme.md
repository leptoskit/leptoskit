<h1 align="center">leptoskit_plugin_print</h1>

<p align="center">
  <strong>leptoskit_plugin_print</strong> is the leptoskit plugin for owned-window print dialog presentation.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_print"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_print"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_print"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_print.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_print"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_print/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Presents the system print dialog for HTML markup or a PDF URI. Completion means the dialog finished or the platform accepted a print job. It never confirms that paper left a printer.

Enable the `guest` feature for native Wasm clients. Select `print` in `native.plugins`. iOS uses `UIPrintInteractionController`; macOS uses `NSPrintOperation`; Android uses `PrintManager`. Browser and desktop-without-dialog hosts report Unsupported or Unavailable for this MVP.

## How it fits

Add the crate, select `print` in `native.plugins`, and call `guest::Print` from Wasm guests. Hosts without a print dialog register `Unsupported`.

## Use it

```rust
use leptoskit_plugin_print::{Job, guest::Print};

let print = Print::new()?;
let _ = print.availability().await?;
print.print(&Job::html("<p>Hello</p>")?).await?;
```

## Learn more

See the [plugin support matrix](https://leptoskit.dev/platforms/plugin-inventory.html) and [runtime plugins](https://leptoskit.dev/platforms/plugins.html).

<!-- {=p_printSummary} -->

Add `leptoskit_plugin_print` as a direct dependency and select the `print` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Guests enable the `guest` feature and reuse `guest::Print`. Completion means the print dialog finished or the platform accepted a job; it never confirms paper left a printer.

<!-- {/p_printSummary} -->
