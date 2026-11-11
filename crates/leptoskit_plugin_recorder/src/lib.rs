//! <!-- {=p_recorderSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_recorder` as a direct dependency and select the
//! `recorder` plugin in `native.plugins`; supported generated hosts discover it
//! from Cargo metadata. Generated Windows, Linux, and terminal hosts currently
//! omit runtime plugins, so use explicit registration in a custom host there.
//! Guests enable the `guest` feature and reuse `guest::Recorder` rather than
//! resolving the plugin on every call. iOS records through `AVAudioEngine` and
//! Android through `AudioRecord` with a `MediaCodec` AAC encoder; the app
//! supplies its `NSMicrophoneUsageDescription` purpose string and, for Android
//! background recording, runs the `background` plugin's microphone-typed
//! foreground service.
//!
//! <!-- {/p_recorderSummary} -->

use leptoskit_bridge::MethodId;

mod segment;
mod types;
mod wire;

pub use segment::*;
pub use types::*;

/// Stable native registration name.
pub const NAME: &str = "recorder";
/// Microphone permission state, empty request; never prompts.
pub const PERMISSION: MethodId = MethodId(0);
/// Shows the system's microphone permission screen, empty request.
pub const REQUEST_PERMISSION: MethodId = MethodId(1);
/// Starts continuous recording, as [`Options`] encodes it. Replies with a
/// [`Recovery`] report for leftover files from an earlier session.
pub const START: MethodId = MethodId(2);
/// Stops recording and finishes the current segment so it is delivered.
pub const STOP: MethodId = MethodId(3);
/// Reports the current state, empty request.
pub const STATUS: MethodId = MethodId(4);
/// Waits for the next finished [`Segment`], empty request; the wait is
/// cancellable, and a reply with no segment ends the stream.
pub const NEXT_SEGMENT: MethodId = MethodId(5);
/// Delivers leftover files from an earlier session without starting to record,
/// empty request.
pub const RECOVER: MethodId = MethodId(6);
/// Exported independently grantable operations.
pub const METHODS: &[MethodId] = &[
	PERMISSION,
	REQUEST_PERMISSION,
	START,
	STOP,
	STATUS,
	NEXT_SEGMENT,
	RECOVER,
];

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest;

/// Continuous background recording does not exist in the browser: the page's
/// own lifetime is its background lifetime, and a hidden tab loses its
/// microphone stream.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use leptoskit_bridge::Error;

	use super::Options;
	use super::Permission;
	use super::Recovery;
	use super::Segment;
	use super::Status;

	/// Browser-side recorder client; every operation reports
	/// [`Error::Unavailable`].
	#[derive(Debug, Clone, Copy, Default)]
	pub struct Recorder;
	impl Recorder {
		/// Background recording does not exist in the browser.
		pub const fn new() -> Self {
			Self
		}

		/// Unavailable: the browser has no continuous microphone pipeline.
		pub async fn permission(self) -> Result<Permission, Error> {
			Err(Error::Unavailable)
		}

		/// Unavailable: the browser has no continuous microphone pipeline.
		pub async fn request_permission(self) -> Result<Permission, Error> {
			Err(Error::Unavailable)
		}

		/// Unavailable: the browser has no continuous microphone pipeline.
		pub async fn start(self, _options: &Options) -> Result<Recovery, Error> {
			Err(Error::Unavailable)
		}

		/// Unavailable: nothing is recording.
		pub async fn stop(self) -> Result<(), Error> {
			Err(Error::Unavailable)
		}

		/// Unavailable: there is no recording state.
		pub async fn status(self) -> Result<Status, Error> {
			Err(Error::Unavailable)
		}

		/// Unavailable: there is no recording session.
		pub async fn next_segment(self) -> Result<Option<Segment>, Error> {
			Err(Error::Unavailable)
		}

		/// Unavailable: there are no leftover segments.
		pub async fn recover(self) -> Result<Recovery, Error> {
			Err(Error::Unavailable)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn methods_are_dense_and_in_order() {
		for (index, method) in METHODS.iter().enumerate() {
			assert_eq!(method.0 as usize, index);
		}
	}

	#[cfg(not(target_arch = "wasm32"))]
	#[test]
	fn every_method_has_a_declared_constant() {
		assert_eq!(PERMISSION, MethodId(0));
		assert_eq!(REQUEST_PERMISSION, MethodId(1));
		assert_eq!(START, MethodId(2));
		assert_eq!(STOP, MethodId(3));
		assert_eq!(STATUS, MethodId(4));
		assert_eq!(NEXT_SEGMENT, MethodId(5));
		assert_eq!(RECOVER, MethodId(6));
	}
}
