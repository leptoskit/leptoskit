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
//! - [`access`]: roles, token scopes, and the one table that turns them into
//!   permissions.
//! - `tables`, with `server`: the control plane's tables. Clients never see a
//!   row; every operation answers a view type.
//! - [`account`], [`tokens`], [`projects`], [`deployments`], [`operator`]: the
//!   control plane's operations, as every client calls them.
//! - [`workers`]: the protocol build and cluster workers speak, its operations,
//!   and the jobs' payloads.
//! - [`release`]: how a deployment becomes the `ReleaseSpec` a cluster applies,
//!   and the hostname it is served at.
//!
//! Without features it has no server dependencies and compiles for
//! `wasm32-unknown-unknown`. The `[cloud]` table itself lives in
//! [`leptoskit_config::cloud`], which defines the size enums and slugs once;
//! this crate re-exports them.
//!
//! With the `server` feature, it is also the control plane itself
//! ([`server`]): the operations' bodies, identity, sign-in, the CLI's device
//! flow, and the routes, which `leptoskit_cloud_app` serves. The
//! operations' bodies exist only in that build: a crate that links this one
//! for its client, such as the CLI, compiles none of them.
//!
//! Billing, secrets, domains, and managed databases arrive in later slices.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/leptoskit/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/leptoskit/leptoskit/main/docs/src/assets/mark.svg"
)]

pub mod access;
pub mod account;
pub mod archive;
pub mod config;
pub mod deployment;
pub mod deployments;
pub mod operator;
pub mod pricing;
pub mod projects;
pub mod release;
#[cfg(feature = "server")]
pub mod server;
pub mod size;
pub mod slug;
// Tables are server-only: `#[table]` expands by `leptoskit_db`'s unified
// features, so a client build that shares a graph with a database backend
// would otherwise compile rows it can't. Clients see view types instead.
#[cfg(feature = "server")]
pub mod tables;
pub mod tokens;
pub mod workers;

leptoskit_rpc::scopes! {
	/// What the CLI may ask for when someone signs it in with the device
	/// flow. Every control-plane operation requires it of a third-party
	/// access token.
	pub enum OAuthScope {
		/// Act as you in the cloud: see and change your organisations,
		/// projects, environments, and tokens.
		Cloud = "cloud",
	}
}

// Sign-in, the device flow, and the RPC's own bookkeeping keep their
// tables in the control plane's database, beside its own.
#[cfg(feature = "server")]
leptoskit_db::packs!(leptoskit_auth, leptoskit_idp, leptoskit_rpc);

#[cfg(feature = "server")]
leptoskit_db::export_schema!();

/// Every migration of the control plane's database on SQLite, generated with
/// `leptoskit db migration create` and embedded for `leptoskit-cloud-control
/// migrate` to apply.
#[cfg(feature = "server")]
pub static MIGRATIONS: &[leptoskit_db::Migration] = leptoskit_db::migrations!("migrations");

/// The same migrations written for Postgres.
#[cfg(feature = "postgres")]
pub static POSTGRES_MIGRATIONS: &[leptoskit_db::Migration] =
	leptoskit_db::migrations!("migrations-postgres");
