//! `LeptosKit` Cloud's notifications hub: one process per cluster that serves
//! the hosted notifications of every environment on it.
//!
//! An app that declares `[services.notifications]` with the registry crate
//! `leptoskit_notifications` runs no process for it in the cloud. Its
//! environment is a *tenant* of its cluster's hub instead: a database of its
//! own on the cluster's Postgres server, made by the cluster's worker, a
//! token the app and the hub share, and the push settings the environment
//! keeps as its `LEPTOSKIT_NOTIFICATIONS_*` secrets and variables, which
//! only the hub is given.
//!
//! The hub is a worker of the `notifications` role, paired to its cluster
//! with `leptoskit-cloud-worker pair` and a code from `issue_pairing`. It
//! never takes jobs. Every round it asks the control plane's
//! `notification_tenants` what changed among its tenants, given the ones it
//! holds at their versions, and for each it is given it opens the database,
//! migrates it, and starts the service's routes and job workers with that
//! tenant's token and settings; for each it is told to drop, it stops them.
//! Then it says how each is doing, and how many pushes each sent, with
//! `report_notifications`.
//!
//! # Routing
//!
//! [`Hub::router`] answers two kinds of request:
//!
//! - `/_leptoskit/services/notifications/…` on a tenant's hostname: the
//!   cluster's edge sends the service's public routes on each app hostname
//!   here, with the `Host` header as it came, which names the tenant.
//! - `/t/<tenant>/_leptoskit/services/notifications/…`: the app's own calls,
//!   from inside the cluster, name the tenant in the path, as
//!   `LEPTOSKIT_SERVICE_NOTIFICATIONS_URL` says.
//!
//! Either way the request reaches that tenant's router and nothing else, and
//! every operation there wants the tenant's own token or a person the app
//! forwarded with a key derived from it. A forged `Host` or path reaches
//! another tenant's router, which refuses it as it would on the internet.
//!
//! # Usage
//!
//! A push counts when its provider accepts it: a `push_deliveries` row in
//! the `accepted` state. The hub counts each tenant's by when they were
//! accepted, from the cursor the control plane holds to a minute ago, so a
//! delivery still being written is counted in the next round, and the
//! control plane counts a range only once, whichever hub reports it.

#![cfg(unix)]

pub mod cli;
mod hub;
mod usage;

pub use hub::Connect;
pub use hub::Hub;
pub use hub::Open;
pub use hub::Pending;
pub use hub::Plane;
pub use hub::Round;
pub use usage::Cursor;
pub use usage::LAG;
pub use usage::count;
