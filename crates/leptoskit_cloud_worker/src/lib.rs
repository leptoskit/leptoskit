//! The worker of a cloud: the program that does what a control plane asks.
//!
//! A worker pairs with a control plane once, with a one-use code an
//! operator issues, and keeps the key it is answered in its state
//! directory. From then on it asks for work: [`Worker::run`] claims one job
//! at a time, renews its lease while it runs, sends its log, and reports
//! how it ended.
//!
//! This crate holds the loop every role shares and the `build` role. The
//! `cluster` role, native builds and store publishing aren't built.
//!
//! # What a build is here
//!
//! For a `build_server` job the worker downloads the source archive,
//! checks its size and SHA-256, unpacks it into a fresh directory, and runs
//! a [`Builder`]. The one builder there is, [`CommandBuilder`], runs a
//! program the operator names and reads what it built from a file the
//! program writes. The directory is deleted when the build ends, and one
//! that a worker which was killed left behind is deleted when it next
//! starts.
//!
//! **The worker doesn't isolate the build.** The program runs as the
//! worker's own user, with a cleared environment and nothing else between
//! it and the machine: it can read the worker's key and reach the network,
//! a process it leaves outside its process group outlives it, and every
//! variable the operator passes it reaches every build.
//! Building other people's code safely needs the program to do the work
//! inside a sandbox of its own, a virtual machine or a container that is
//! thrown away afterwards and is handed only the source directory. That
//! sandbox isn't part of this crate.
//!
//! # Crashes
//!
//! Before a job runs the worker writes a marker naming it, and before it
//! reports an outcome it writes the outcome down. Started again, it sends
//! an outcome it had saved. A marker with no outcome means the job's effect
//! is unknown: a build's only effect is an image named by its digest, so
//! the marker is dropped and the control plane gives the build to a worker
//! again when its lease runs out; any other kind of job is never run
//! twice, and the worker refuses to start until an operator has reconciled
//! it.

#![cfg(unix)]

mod build;
mod client;
mod command;
mod logs;
mod run;
mod state;

pub mod cli;

pub use build::BuildJob;
pub use build::Builder;
pub use build::Stop;
pub use client::CallError;
pub use client::Control;
pub use client::Key;
pub use client::OriginError;
pub use command::BuildRequest;
pub use command::CommandBuilder;
pub use command::kill_running_build;
pub use logs::LogSink;
pub use run::Ended;
pub use run::RunError;
pub use run::Settings;
pub use run::Step;
pub use run::Worker;
pub use state::Identity;
pub use state::Marker;
pub use state::Saved;
pub use state::State;
pub use state::StateError;
