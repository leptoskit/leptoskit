//! The cloud's control plane, as a program: `leptoskit-cloud-control`.
//!
//! The control plane itself, its operations, sign-in, and routes, is
//! [`leptoskit_cloud`]'s `server` feature. This crate reads the operator's
//! environment, refuses a configuration that is incomplete or unsafe, and
//! runs the commands: `serve`, `migrate`, and `operator`. The readme lists
//! every variable it reads.

#[cfg(feature = "server")]
pub mod server;
