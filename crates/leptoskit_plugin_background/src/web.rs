//! Background execution is unavailable in the browser: the page's own
//! lifetime is its background lifetime, so there is no persistent service to
//! keep alive.

use super::*;

/// Browser-side background client; every operation reports
/// [`Error::Unavailable`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Background;
impl Background {
	/// Background work does not exist in the browser.
	pub const fn new() -> Self {
		Self
	}

	/// Unavailable: the browser already runs for the page's lifetime.
	pub async fn start(self, _options: &Options) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Unavailable: there is no foreground service to stop.
	pub async fn stop(self) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Unavailable: there is no background execution state.
	pub async fn status(self) -> Result<Status, Error> {
		Err(Error::Unavailable)
	}

	/// Unavailable: battery optimisation is an Android concept.
	pub async fn request_ignore_battery_optimizations(self) -> Result<BatteryOptimization, Error> {
		Err(Error::Unavailable)
	}
}
