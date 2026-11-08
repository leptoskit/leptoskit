//! Cached native background client.
use super::*;

/// Resolves the host once, then uses numeric method identifiers.
#[derive(Debug, Clone, Copy)]
pub struct Background(leptoskit_bridge::guest::Client);
impl Background {
	/// Resolves an explicitly installed host plugin.
	pub fn new() -> Result<Self, Error> {
		leptoskit_bridge::guest::Client::new(NAME).map(Self)
	}

	/// Starts the foreground service and its periodic wake on Android, or
	/// schedules background tasks on iOS. Repeated calls replace the previous
	/// configuration; there is one background session per app.
	pub async fn start(self, options: &Options) -> Result<(), Error> {
		self.empty(START, &options.encode()?).await
	}

	/// Stops the foreground service and cancels scheduled background tasks.
	pub async fn stop(self) -> Result<(), Error> {
		self.empty(STOP, &[]).await
	}

	/// Reports the running state, last wake, and battery optimisation.
	pub async fn status(self) -> Result<Status, Error> {
		Status::decode(&self.0.call(STATUS, &[])?.await?)
	}

	/// Asks the user to exempt the app from battery optimisation, then reports
	/// the resulting state. The system dialog may outlive cancellation.
	pub async fn request_ignore_battery_optimizations(self) -> Result<BatteryOptimization, Error> {
		BatteryOptimization::decode(
			&self
				.0
				.call(REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, &[])?
				.await?,
		)
	}

	async fn empty(self, method: MethodId, payload: &[u8]) -> Result<(), Error> {
		if self.0.call(method, payload)?.await?.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}
}
