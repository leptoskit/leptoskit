//! <!-- {=logSummary|trim|linePrefix:"//! ":true} -->
//!
//! It sends leptoskit's own records to the platform's log: logcat on Android,
//! the unified log (`os_log`) on iOS and macOS, standard error on Linux and
//! Windows, and the console in a browser. The runtime installs it when an app
//! starts, with a panic hook, so a failure on a device says which plugin, call,
//! node, and payload it was about. Nothing leaves the device.
//!
//! <!-- {/logSummary} -->
//!
//! The runtime speaks the [`log`] facade, so the records of the libraries it
//! uses, such as wgpu's, reach the same place. Each record's target names the
//! part of leptoskit it comes from, and becomes the logcat tag or the
//! `os_log` category:
//!
//! | Target                     | What it covers                                        |
//! | -------------------------- | ----------------------------------------------------- |
//! | [`target::BRIDGE`]         | App creation, the guest's start and failures, lifecycle |
//! | [`target::WASM`]           | The interpreter: traps, and which export and import    |
//! | [`target::RENDER`]         | Commits and the props the guest sends for each node    |
//! | [`target::GPU`]            | Native drawing surfaces and the GPU device            |
//! | [`target::PANIC`]          | Panics, with their location, thread, and backtrace    |
//! | `leptoskit/plugin/<name>`  | One plugin's registration and failed calls ([`target::plugin`]) |
//!
//! Release builds keep warnings and errors; debug builds add `info`. An app
//! raises or lowers that with `[native] log-level` in `leptoskit.toml`, which
//! the CLI passes to this crate's build as `LEPTOSKIT_LOG_LEVEL`.
//!
//! [`excerpt::Excerpt`] cuts a bounded, UTF-8-safe window from a payload around
//! the byte a decoder failed at, so a record can show the bytes that failed
//! without filling the log.
//!
//! There is no telemetry: records are written to the platform's own log and
//! nowhere else.

use std::sync::Once;

use log::LevelFilter;
use log::Log;
use log::Metadata;
use log::Record;

#[cfg(feature = "testing")]
pub mod capture;
pub mod excerpt;
mod panic;
mod sink;

pub use excerpt::Excerpt;
pub use excerpt::MAX_EXCERPT_BYTES;

/// The targets leptoskit's records use. They name the part of the runtime a
/// record comes from, and are the logcat tags and `os_log` categories.
pub mod target {
	/// App creation, the guest thread's start and failures, and lifecycle.
	pub const BRIDGE: &str = "leptoskit/bridge";
	/// The Wasm interpreter: traps, and the export and import they came from.
	pub const WASM: &str = "leptoskit/wasm";
	/// The guest's commits, and the props it sends for each node.
	pub const RENDER: &str = "leptoskit/render";
	/// Native drawing surfaces and the GPU device behind them.
	pub const GPU: &str = "leptoskit/gpu";
	/// Panics, from any thread.
	pub const PANIC: &str = "leptoskit/panic";
	/// What [`plugin`] puts before a plugin's name.
	pub const PLUGIN_PREFIX: &str = "leptoskit/plugin/";

	/// The target of one plugin's records, such as `leptoskit/plugin/http`.
	pub fn plugin(name: &str) -> String {
		format!("{PLUGIN_PREFIX}{name}")
	}
}

/// The level a build logs at when `leptoskit.toml` sets none: `info` in a
/// debug build, `warn` in a release build.
pub const DEFAULT_LEVEL: LevelFilter = if cfg!(debug_assertions) {
	LevelFilter::Info
} else {
	LevelFilter::Warn
};

/// The most detailed level this build writes: `[native] log-level` from
/// `leptoskit.toml` when the CLI set it, otherwise [`DEFAULT_LEVEL`].
pub fn level() -> LevelFilter {
	option_env!("LEPTOSKIT_LOG_LEVEL")
		.and_then(|level| level.trim().parse().ok())
		.unwrap_or(DEFAULT_LEVEL)
}

static LOGGER: Logger = Logger;
static INSTALL: Once = Once::new();

/// Sends records to the platform's log at [`level`] and installs the panic
/// hook. The first call does the work; later calls return at once.
///
/// When the process already has a `log` logger, such as one an app
/// installed itself, records keep going there and its level is left alone;
/// only the panic hook is added. The hook runs before any hook installed
/// earlier, which still runs.
pub fn install() {
	INSTALL.call_once(|| {
		let ours = log::set_logger(&LOGGER).is_ok();

		if ours {
			log::set_max_level(level());
		}

		panic::install();

		if ours {
			log::info!(
				target: target::BRIDGE,
				"leptoskit {} logs at {} and above",
				env!("CARGO_PKG_VERSION"),
				level(),
			);
		}
	});
}

/// The logger [`install`] sets.
struct Logger;

impl Log for Logger {
	fn enabled(&self, metadata: &Metadata<'_>) -> bool {
		metadata.level() <= log::max_level()
	}

	fn log(&self, record: &Record<'_>) {
		if !self.enabled(record.metadata()) {
			return;
		}

		#[cfg(feature = "testing")]
		if capture::keep(record) {
			return;
		}

		sink::write(
			record.level(),
			tag(record.target()),
			&record.args().to_string(),
		);
	}

	fn flush(&self) {}
}

/// The tag a record's target is written under: leptoskit's own targets as
/// they are, and the crate a library's record comes from otherwise, so
/// `wgpu_hal::vulkan::instance` reads as `wgpu_hal`.
pub fn tag(target: &str) -> &str {
	if target.starts_with("leptoskit/") {
		return target;
	}

	target.split("::").next().unwrap_or(target)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn leptoskit_targets_keep_their_name_and_libraries_their_crate() {
		assert_eq!(tag(target::BRIDGE), "leptoskit/bridge");
		assert_eq!(tag(&target::plugin("http")), "leptoskit/plugin/http");
		assert_eq!(tag("wgpu_hal::vulkan::instance"), "wgpu_hal");
		assert_eq!(tag("naga"), "naga");
	}

	#[test]
	fn the_default_level_follows_the_build() {
		let expected = if cfg!(debug_assertions) {
			LevelFilter::Info
		} else {
			LevelFilter::Warn
		};
		assert_eq!(DEFAULT_LEVEL, expected);
	}
}
