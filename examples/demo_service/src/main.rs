//! The demo service's entry point.
//!
//! The process is Unix only, deliberately: its locks are `flock`, its
//! daily reset re-executes through `execve`, and it is deployed on Linux.
//! The whole implementation lives behind `cfg(unix)` in [`process`], and
//! on any other platform the binary refuses immediately — a nonzero exit
//! before reading configuration, binding, or creating state — instead of
//! pretending to serve.

#[cfg(unix)]
mod process;

#[cfg(unix)]
fn main() -> std::process::ExitCode {
	process::main()
}

#[cfg(not(unix))]
fn main() -> std::process::ExitCode {
	eprintln!(
		"the demo service refuses to start: this daily-reset service requires Unix flock/exec and \
		 is deployed on Linux; it is unsupported on this platform"
	);
	std::process::ExitCode::FAILURE
}
