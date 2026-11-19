<h1 align="center">leptoskit_admin_pages</h1>

<p align="center">
  <a href="https://leptoskit.dev/"><img src="https://raw.githubusercontent.com/leptoskit/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_admin_pages</strong> holds the leptoskit admin's pages: rendered by <code>leptoskit_admin</code> on the server and hydrated in the browser from the same component trees.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#build-the-client"><strong>Build the client</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_admin_pages"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_admin_pages"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_admin_pages"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_admin_pages.svg"></a>
  <a href="https://docs.rs/leptoskit_admin_pages"><img alt="docs.rs" src="https://docs.rs/leptoskit_admin_pages/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

You don't depend on this crate yourself: [`leptoskit_admin`](https://docs.rs/leptoskit_admin) does. It holds the admin's component trees, which build twice. `leptoskit_admin` links the library with the `ssr` feature and renders each page on the server from a small model. `leptoskit build` compiles the same library to Wasm with `hydrate`, and the browser reads the model back from the page, builds the same tree, and attaches to the markup. The theme, the account menu, search, and the keyboard shortcuts (⌘K or Ctrl+K to find a screen, Shift+W for Workings mode) are Rust on both sides, with no hand-written script.

Pages move here one at a time. Until a page has moved, `leptoskit_admin` renders it the old way, from the pieces this crate shares: the colour tokens, the components, the Workings drawer, and the charts.

## Build the client

```sh
cd crates/leptoskit_admin_pages
leptoskit build --release --platform web --out /srv/notes-admin/client
```

Then give the admin that directory with `Admin::builder(…).client_dir("/srv/notes-admin/client")`. It serves the client from `/_leptoskit/admin/pkg/`. Without a client the pages still render, and stay as the server drew them.

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_admin_pages)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Run an admin for your site](https://leptoskit.dev/docs/guide/32-admin.html)
