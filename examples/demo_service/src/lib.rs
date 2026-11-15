//! One process image serves exactly one UTC day's public demo generation:
//! a main site with sign-in, its identity provider, the analytics service,
//! and the deploy console's admin, all backed by durable daily state under
//! the service volume.
//!
//! The lifecycle is deliberate:
//!
//! - **One generation per process image.** The opened generation lives in a
//!   [`OnceLock`] and is never swapped. A new day, a failed initialization, or
//!   a crash is answered by replacing the process image with the same binary
//!   through `execve`, which keeps the PID and closes every file descriptor, so
//!   the kernel releases the generation's locks only once the old execution has
//!   fully stopped.
//! - **Publication is a durable marker, not a rename.** A generation is built
//!   at its stable final path (`gen/<UTC day>.f<format>`) under a kernel build
//!   lock, and becomes visible to other processes only when its fsynced
//!   `published` marker names a matching format and schema fingerprint.
//! - **Locks are files and live as long as the process.** `build.lock`
//!   serializes builders and collectors; a generation's `lifetime.lock` is held
//!   shared by every process serving it and taken exclusively, non-blocking,
//!   before anything deletes it; `worker.lock` names the one process that runs
//!   the control service's worker. All three are held by owned file descriptors
//!   retained until exec or exit, so an aborted async task can never release
//!   them while a writer is still running.
//!
//! Visitor onboarding (personas, inbox, limits, pages) is a later task:
//! this slice has no public sign-in and the mailer is a sink.

pub mod admin;
pub mod barrier;
pub mod clock;
pub mod config;
pub mod dispatch;
pub mod generation;
pub mod locks;
pub mod marker;
pub mod secrets;
pub mod shutdown;
pub mod site;
pub mod worker;

use std::sync::OnceLock;

use crate::config::Authorities;

/// The authorities the process answers for, set once at startup before the
/// listener exists, so the fallback already rejects foreign hosts with 421.
pub static AUTHORITIES: OnceLock<Authorities> = OnceLock::new();

/// The opened generation. Set exactly once, when initialization commits:
/// one process image serves one generation and never replaces it.
pub static GENERATION: OnceLock<generation::Generation> = OnceLock::new();

/// Why the process stops serving: a supervisor's SIGTERM, or its own daily
/// reset, which replaces the image through `execve`. Termination is
/// authoritative: a SIGTERM wins over a reset in whichever order they
/// arrive, and the exit-or-exec choice reads the stop state once more
/// after the runtime has stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
	/// A deliberate stop (`SIGTERM`): leave with status 0.
	Terminate,
	/// The day changed, or a bounded retry is due: re-execute the same
	/// binary in the same PID.
	Reset,
}
