//! The shared vocabulary of `LeptosKit` Cloud, the hosted platform that runs
//! any leptoskit app from the `leptoskit.toml` it already has.
//!
//! Every part of the cloud speaks these types: the CLI (`lk cloud`), the
//! dashboard, which compiles to wasm, the control plane, and the build and
//! cluster workers. This crate is the foundation they build on:
//!
//! - [`slug`]: the validated names organisations, projects, and environments go
//!   by.
//! - [`size`]: process sizes and their CPU and memory, replica scale, sleep,
//!   database sizes, backups, and regions.
//! - [`deployment`]: a deployment's states and the transitions between them,
//!   the typed reason a deployment failed, and where its source came from.
//! - [`archive`]: the source archive's digest and bounds, and, with the
//!   `archive` feature, packing a project and unpacking it safely.
//! - [`pricing`]: a price catalogue and the monthly estimate for a `[cloud]`
//!   table, in integer minor units.
//!
//! Without features it has no server dependencies and compiles for
//! `wasm32-unknown-unknown`. The `[cloud]` table itself lives in
//! [`leptoskit_config::cloud`], which defines the size enums and slugs once;
//! this crate re-exports them.
//!
//! The cloud's tables, operations, server, and workers arrive in later
//! slices; nothing here talks to a network.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/leptoskit/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/leptoskit/leptoskit/main/docs/src/assets/mark.svg"
)]

pub mod archive;
pub mod deployment;
pub mod pricing;
pub mod size;
pub mod slug;
