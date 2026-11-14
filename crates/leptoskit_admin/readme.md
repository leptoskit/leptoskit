<h1 align="center">leptoskit_admin</h1>

<p align="center">
  <a href="https://leptoskit.dev/"><img src="https://raw.githubusercontent.com/leptoskit/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_admin</strong> is one admin web app for a leptoskit site: its analytics, its infrastructure, and lessons in both, signed in through the site's own sign-in.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#try-it"><strong>Try it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_admin"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_admin"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_admin"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_admin.svg"></a>
  <a href="https://docs.rs/leptoskit_admin"><img alt="docs.rs" src="https://docs.rs/leptoskit_admin/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

The admin is a separate web app from your site, with its own address and its own deploy. Nobody gets a password for it: people press **Continue with Notes**, sign in on your site, and come back. Only accounts your site gives the `admin` role get in; everyone else sees who they signed in as and how admins are chosen.

Inside, there are three sections:

- **Infrastructure** works against the deployment service's control API. Plan, apply, deploy, first release, maintenance, roll back, health checks, and backup verification all run as operations, recorded under the signed-in admin's name, with progress streamed live to the page.
- **Analytics** teaches until your app sends its first typed event, rather than drawing charts from nothing.
- **Learn** has short lessons that tick themselves when they become true of your projects: you applied a plan, a backup restored, a health check passed.

Every card has a **Workings** drawer that walks the thing on screen back to where it comes from: the `leptoskit.toml` lines that declare it, the `main.tf.json` they generate, the plan, the command the worker runs, and the check that says it's done. `W` opens it; `Shift+W` keeps the sources showing on every card.

## How it fits

The main site runs `leptoskit_auth` with a `Roles` hook and `leptoskit_idp`, and registers the admin as a first-party client. The admin is an OpenID Connect client of it (`oidc:main`, reading roles from `userinfo`), so a role change on the main site applies at the admin's next sign-in. The admin keeps its own small SQLite database for sessions, the roles it was last given, and each person's preferences.

Infrastructure actions go through `leptoskit_deploy_console`'s `control::Service`, the same service `lk console serve` runs, so the admin and the CLI see the same operations.

## Use it

Run it from the CLI:

```sh
LEPTOSKIT_ADMIN_CLIENT_SECRET=… LEPTOSKIT_ADMIN_KEYS=… LEPTOSKIT_CONTROL_KEY=… \
  lk admin serve --project /srv/notes \
  --url https://admin.notes.example \
  --main-site https://notes.example --client-id <client id>
```

Or mount it in your own binary:

```rust,ignore
use leptoskit_admin::Admin;
use leptoskit_admin::MainSite;

let admin = Admin::builder("Notes")
	.origin("https://admin.notes.example")
	.main_site(MainSite::new("https://notes.example", client_id, client_secret))
	.keys(leptoskit_auth::Keys::from_env("LEPTOSKIT_ADMIN_KEYS")?)
	.data_dir("/var/lib/notes-admin")
	.control(service)
	.build()
	.await?;
axum::serve(listener, admin.router()).await?;
```

`.roles(["admin", "ops"])` accepts other roles, `.environment(Environment::Staging)` labels a staging admin, and `.database(db)` uses a database you opened instead of `.data_dir`.

## Try it

`examples/admin_demo` runs a main site and its admin side by side, with two people: Ada, an admin, and Grace, who isn't.

```sh
cargo run -p admin_demo                  # operations are simulated
cargo run -p admin_demo -- --executor lk # the real `lk` commands
```

Open `http://127.0.0.1:4801/_leptoskit/admin`. The simulated executor contacts no cloud provider: it runs the real generator and compares the result with what it last "applied", and says it's a simulation in every operation. With `--executor lk`, plans run `lk infra plan` for real and fail without provider tokens, which the admin shows as it is.

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_admin)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Run an admin for your site](https://leptoskit.dev/docs/guide/32-admin.html)
