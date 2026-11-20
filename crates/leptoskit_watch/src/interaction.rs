//! Watch haptic cues (`WKHapticType` on watchOS / Wear tactile feedback),
//! wrist & hardware button gestures (Double Tap, Wrist Flick, Action Button),
//! and per-platform capability descriptors.

use crate::WatchKind;

/// Tactile watch feedback patterns matching `WKHapticType` and Wear OS haptic
/// effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchHaptic {
	/// Subtle crown / selection click.
	Click,
	/// Upward step cue.
	DirectionUp,
	/// Downward step cue.
	DirectionDown,
	/// Positive confirmation.
	Success,
	/// Error / rejected action.
	Failure,
	/// Retry prompt.
	Retry,
	/// Workout / timer started.
	Start,
	/// Workout / timer stopped.
	Stop,
	/// Incoming notification tap.
	Notification,
}

/// Hardware button and one-handed wrist gestures on smartwatches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchGesture {
	/// One-handed index-thumb double tap (Apple Watch Series 9+ / Ultra 2+ /
	/// Wear OS pinch gestures).
	DoubleTap,
	/// Wrist flick dismissal gesture.
	WristFlick,
	/// Pressing the Digital Crown / primary rotary button.
	CrownPress,
	/// Pressing the side button.
	SideButtonPress,
	/// Pressing a dedicated programmable Action Button (Apple Watch Ultra /
	/// rugged Wear OS watches).
	ActionButtonPress,
}

/// Platform feature matrix for a [`WatchKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct WatchCapabilities {
	/// Rotary input (Digital Crown or Wear OS bezel/rotary side button).
	pub digital_crown: bool,
	/// Always-On Display / ambient mode lifecycle.
	pub always_on_display: bool,
	/// Live workout / exercise sessions.
	pub workouts: bool,
	/// Companion phone sync (`WatchConnectivity` / Wear Data Layer).
	pub companion_sync: bool,
	/// Watch face complications.
	pub complications: bool,
	/// Wear OS tiles (`WatchTile`).
	pub tiles: bool,
	/// watchOS Smart Stack glances (`WatchGlance`).
	pub glances: bool,
	/// One-handed Double Tap gesture.
	pub double_tap: bool,
}

impl WatchCapabilities {
	/// Returns the hardware & OS surface matrix for `kind`.
	#[must_use]
	pub const fn for_kind(kind: WatchKind) -> Self {
		match kind {
			WatchKind::WatchOs => {
				Self {
					digital_crown: true,
					always_on_display: true,
					workouts: true,
					companion_sync: true,
					complications: true,
					tiles: false,
					glances: true,
					double_tap: true,
				}
			}
			WatchKind::WearOs => {
				Self {
					digital_crown: true,
					always_on_display: true,
					workouts: true,
					companion_sync: true,
					complications: true,
					tiles: true,
					glances: false,
					double_tap: true,
				}
			}
			WatchKind::HarmonyOs => {
				Self {
					digital_crown: true,
					always_on_display: true,
					workouts: true,
					companion_sync: true,
					complications: true,
					tiles: true,
					glances: false,
					double_tap: false,
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn watch_capabilities_reflect_watchos_and_wearos_surfaces() {
		let apple = WatchCapabilities::for_kind(WatchKind::WatchOs);
		assert!(apple.glances);
		assert!(!apple.tiles);
		assert!(apple.digital_crown);
		assert!(apple.workouts);

		let wear = WatchCapabilities::for_kind(WatchKind::WearOs);
		assert!(wear.tiles);
		assert!(!wear.glances);
		assert!(wear.always_on_display);
		assert!(wear.companion_sync);
	}
}
