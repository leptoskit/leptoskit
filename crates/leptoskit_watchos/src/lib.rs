//! <!-- {=watchosSummary|trim|linePrefix:"//! ":true} -->
//!
//! Experimental watchOS backend for `leptoskit_watch` surfaces. Builds on every
//! host as a recording stub so guests and tests compile; the Swift files under
//! `swift/` name the `WatchKit` / `WidgetKit` entry points a real device host
//! will bind later.
//!
//! <!-- {/watchosSummary} -->
//!
//! # Experimental
//!
//! watchOS packaging, simulator install, and live complication updates are
//! not implemented. Prefer [`WatchOsHost`] in tests and scaffolding; it
//! validates budgets and records presentations without Objective-C.

#![forbid(unsafe_code)]

use leptoskit_watch::RecordingHost;
use leptoskit_watch::WatchAmbient;
use leptoskit_watch::WatchError;
use leptoskit_watch::WatchHost;
use leptoskit_watch::WatchPage;
use leptoskit_watch::WatchPresentation;
use leptoskit_watch::WatchViewport;

/// watchOS host stub: validates surfaces and records presentations.
#[derive(Debug, Clone)]
pub struct WatchOsHost {
	inner: RecordingHost,
}

impl WatchOsHost {
	/// A medium Apple Watch viewport.
	#[must_use]
	pub fn new() -> Self {
		Self {
			inner: RecordingHost::new(WatchViewport::watch_os_medium()),
		}
	}

	/// Starts with an explicit viewport (for unusual faces).
	pub fn with_viewport(viewport: WatchViewport) -> Result<Self, WatchError> {
		if viewport.kind != leptoskit_watch::WatchKind::WatchOs {
			return Err(WatchError::Unavailable(
				"WatchOsHost requires a watchOS viewport".into(),
			));
		}

		Ok(Self {
			inner: RecordingHost::new(viewport),
		})
	}

	/// Presentations recorded so far.
	#[must_use]
	pub fn presentations(&self) -> &[WatchPresentation] {
		self.inner.presentations()
	}

	/// Runtime capabilities available on watchOS hosts.
	#[must_use]
	pub const fn capabilities() -> WatchOsCapabilities {
		WatchOsCapabilities {
			healthkit_background_delivery: true,
			healthkit_live_workout: true,
			core_motion_batched_sensors: true,
			watch_connectivity: true,
			widgetkit_glances: true,
			complications: true,
		}
	}
}

/// Device capabilities exposed by a watchOS host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct WatchOsCapabilities {
	/// `HKObserverQuery` + `enableBackgroundDelivery`.
	pub healthkit_background_delivery: bool,
	/// `HKWorkoutSession` + `HKLiveWorkoutBuilder` continuous wrist streaming.
	pub healthkit_live_workout: bool,
	/// `CMPedometer` and `CMBatchedSensorManager`.
	pub core_motion_batched_sensors: bool,
	/// `WCSession` companion sync with paired iPhone.
	pub watch_connectivity: bool,
	/// `WidgetKit` Smart Stack glances.
	pub widgetkit_glances: bool,
	/// `WidgetKit` accessory complications.
	pub complications: bool,
}

impl Default for WatchOsHost {
	fn default() -> Self {
		Self::new()
	}
}

impl WatchHost for WatchOsHost {
	fn viewport(&self) -> WatchViewport {
		self.inner.viewport()
	}

	fn present_page(&mut self, page: &WatchPage) -> Result<(), WatchError> {
		self.inner.present_page(page)
	}

	fn present_ambient(&mut self, ambient: &WatchAmbient) -> Result<(), WatchError> {
		self.inner.present_ambient(ambient)
	}

	fn present_app(&mut self, app: &leptoskit_watch::WatchApp) -> Result<(), WatchError> {
		self.inner.present_app(app)
	}
}

/// Stable platform name used in config and the CLI.
pub const PLATFORM: &str = "watchos";

/// Human-readable product name.
pub const DISPLAY_NAME: &str = "watchOS";

#[cfg(test)]
mod tests {
	use leptoskit_native::components::Text;
	use leptoskit_watch::WatchApp;
	use leptoskit_watch::WatchKind;
	use leptoskit_watch::WatchPage;
	use leptoskit_watch::WearableApp;
	use leptoskit_watch::WearableStatus;
	use leptoskit_watch::WearableStream;

	use super::*;

	#[test]
	fn presents_a_minimal_app() {
		let app =
			WatchApp::new("demo", WatchKind::WatchOs).page(WatchPage::new("home", Text("Hi")));
		let mut host = WatchOsHost::new();
		host.present_app(&app).unwrap();
		assert_ne!(host.presentations(), []);
	}

	#[test]
	fn presents_a_wearable_status_app() {
		let app = WearableApp::new("tracker", WatchKind::WatchOs)
			.streams(&[WearableStream::HeartRate, WearableStream::Steps]);
		let mut host = WatchOsHost::new();
		host.present_wearable(&app, &WearableStatus::default())
			.unwrap();
		assert!(WatchOsHost::capabilities().healthkit_live_workout);
		assert_ne!(host.presentations(), []);
	}

	#[test]
	fn rejects_a_wearos_viewport() {
		let viewport = WatchViewport::wear_os_medium();
		assert!(matches!(
			WatchOsHost::with_viewport(viewport),
			Err(WatchError::Unavailable(_))
		));
	}
}
