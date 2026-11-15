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
//! Visitor onboarding is the site's own contract: a browser owns a pass
//! cookie and its pre-minted Ada or Grace, signs in with a real emailed
//! code into a bounded in-memory inbox, and reaches the admin through
//! the identity provider's `roles` scope. `visitors.rs` owns the passes
//! and people, `inbox.rs` the mailer, `return_to.rs` the local-path
//! allowlist, and `site.rs` wires the routes and the pass gate.
//!
//! The runtime is Unix only, deliberately: the locks are `flock`, the
//! daily reset re-executes through `execve`, and the deployment is Linux.
//! Everything below is gated to `cfg(unix)` — on an unsupported platform
//! this library compiles empty and the binary refuses to start rather
//! than emulate any of it.

#[cfg(unix)]
pub mod admin;
#[cfg(unix)]
pub mod barrier;
#[cfg(unix)]
pub mod clock;
#[cfg(unix)]
pub mod config;
#[cfg(unix)]
pub mod dispatch;
#[cfg(unix)]
pub mod generation;
#[cfg(unix)]
pub mod inbox;
#[cfg(unix)]
pub mod locks;
#[cfg(unix)]
pub mod marker;
#[cfg(unix)]
pub mod return_to;
#[cfg(unix)]
pub mod secrets;
#[cfg(unix)]
pub mod shutdown;
#[cfg(unix)]
pub mod site;
#[cfg(unix)]
pub mod visitors;
#[cfg(unix)]
pub mod worker;

#[cfg(unix)]
use std::sync::OnceLock;

#[cfg(unix)]
use crate::config::Authorities;

/// The authorities the process answers for, set once at startup before the
/// listener exists, so the fallback already rejects foreign hosts with 421.
#[cfg(unix)]
pub static AUTHORITIES: OnceLock<Authorities> = OnceLock::new();

/// The opened generation. Set exactly once, when initialization commits:
/// one process image serves one generation and never replaces it.
#[cfg(unix)]
pub static GENERATION: OnceLock<generation::Generation> = OnceLock::new();

/// Why the process stops serving: a supervisor's SIGTERM, or its own daily
/// reset, which replaces the image through `execve`. Termination is
/// authoritative: a SIGTERM wins over a reset in whichever order they
/// arrive, and the exit-or-exec choice reads the stop state once more
/// after the runtime has stopped.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
	/// A deliberate stop (`SIGTERM`): leave with status 0.
	Terminate,
	/// The day changed, or a bounded retry is due: re-execute the same
	/// binary in the same PID.
	Reset,
}
