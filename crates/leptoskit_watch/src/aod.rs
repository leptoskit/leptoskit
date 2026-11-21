//! Always-On Display (AOD) / Ambient Mode lifecycle (`isLuminanceReduced` on
//! watchOS, `AmbientLifecycleObserver` on Wear OS) and Smart Stack / Tile
//! relevance hints.

use crate::WatchError;

/// Current Always-On Display / ambient hardware state reported by the watch OS.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct AmbientModeState {
	/// `true` while the watch screen is in low-power Always-On / ambient mode.
	pub is_ambient: bool,
	/// `true` when luminance is reduced (watchOS `isLuminanceReduced`).
	pub reduced_luminance: bool,
	/// `true` when the display requires OLED burn-in protection (hollow
	/// strokes, black background, shifted pixels on Wear OS).
	pub burn_in_protection: bool,
	/// `true` on low-bit ambient screens (no anti-aliasing/grayscale).
	pub low_bit_ambient: bool,
}

/// App policy for how a watch page renders during Always-On Display mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AmbientPolicy {
	/// Allow periodic low-frequency updates while dimmed.
	pub allow_updates_in_ambient: bool,
	/// Minimum seconds between ambient refreshes (`1..=3600`, typically 60s).
	pub min_refresh_interval_secs: u32,
	/// Target foreground opacity in ambient mode (`0.1..=1.0`).
	pub dim_opacity: f32,
}

impl Default for AmbientPolicy {
	fn default() -> Self {
		Self {
			allow_updates_in_ambient: true,
			min_refresh_interval_secs: 60,
			dim_opacity: 0.65,
		}
	}
}

impl AmbientPolicy {
	/// Validates refresh interval (`1..=3600`) and opacity (`0.1..=1.0`).
	///
	/// # Errors
	///
	/// Returns [`WatchError::InvalidState`] on out-of-range settings.
	pub fn validate(self) -> Result<Self, WatchError> {
		if self.min_refresh_interval_secs == 0
			|| self.min_refresh_interval_secs > 3600
			|| !self.dim_opacity.is_finite()
			|| !(0.1..=1.0).contains(&self.dim_opacity)
		{
			return Err(WatchError::InvalidState(
				"ambient policy requires refresh interval 1..=3600s and dim_opacity 0.1..=1.0"
					.into(),
			));
		}
		Ok(self)
	}
}

/// Relevance scoring hint for watchOS Smart Stack glances and Wear OS tiles.
#[derive(Debug, Clone, PartialEq)]
pub struct RelevanceHint {
	/// Normalized relevance score (`0.0..=1.0`).
	pub score: f32,
	/// Start of relevance window (Unix epoch seconds).
	pub valid_from_epoch_secs: u64,
	/// End of relevance window (Unix epoch seconds,
	/// `>= valid_from_epoch_secs`).
	pub valid_until_epoch_secs: u64,
	/// Short human-readable explanation (for diagnostics and Smart Stack logs).
	pub reason: String,
}

impl RelevanceHint {
	/// Validates score bounds (`0.0..=1.0`) and chronological window.
	///
	/// # Errors
	///
	/// Returns [`WatchError::InvalidState`] on invalid score or window.
	pub fn new(
		score: f32,
		valid_from_epoch_secs: u64,
		valid_until_epoch_secs: u64,
		reason: impl Into<String>,
	) -> Result<Self, WatchError> {
		if !score.is_finite()
			|| !(0.0..=1.0).contains(&score)
			|| valid_until_epoch_secs < valid_from_epoch_secs
		{
			return Err(WatchError::InvalidState(
				"relevance hint requires score in 0.0..=1.0 and valid_until >= valid_from".into(),
			));
		}
		let reason = reason.into();
		if reason.len() > 128 || reason.chars().any(char::is_control) {
			return Err(WatchError::InvalidState(
				"relevance hint reason must be <= 128 non-control bytes".into(),
			));
		}
		Ok(Self {
			score,
			valid_from_epoch_secs,
			valid_until_epoch_secs,
			reason,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ambient_policy_and_relevance_validate() {
		assert!(AmbientPolicy::default().validate().is_ok());
		assert!(
			AmbientPolicy {
				min_refresh_interval_secs: 0,
				..AmbientPolicy::default()
			}
			.validate()
			.is_err()
		);
		let hint = RelevanceHint::new(0.85, 1000, 2000, "Upcoming flight gate").unwrap();
		assert_eq!(hint.reason, "Upcoming flight gate");
		assert!(RelevanceHint::new(1.5, 1000, 2000, "Bad").is_err());
		assert!(RelevanceHint::new(0.5, 2000, 1000, "Backwards").is_err());
	}
}
