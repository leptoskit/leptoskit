//! Digital Crown (watchOS) and rotary bezel / side-button scroll (Wear OS)
//! input configuration, events, and bounded value accumulator.

use crate::WatchError;

/// Rotary sensitivity preset matching watchOS `DigitalCrownRotation` and Wear
/// OS rotary scroll tuning.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CrownSensitivity {
	/// Fine precision (for pickers and timers).
	Low,
	/// Balanced default.
	#[default]
	Medium,
	/// Fast traversal (for long lists).
	High,
}

/// Configuration for a Digital Crown or rotary bezel binding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrownConfig {
	/// Minimum bound.
	pub min: f64,
	/// Maximum bound (strictly greater than `min`).
	pub max: f64,
	/// Detent step size (positive, finite).
	pub step: f64,
	/// Input sensitivity.
	pub sensitivity: CrownSensitivity,
	/// Trigger subtle haptic detents when crossing `step` boundaries.
	pub haptic_detents: bool,
	/// Wrap around from `max` back to `min` (useful for circular dials).
	pub wrap: bool,
}

impl CrownConfig {
	/// Builds and validates a rotary range `[min, max]` with detent `step`.
	///
	/// # Errors
	///
	/// Returns [`WatchError::InvalidState`] when bounds or step are nonfinite,
	/// `min >= max`, or `step <= 0.0`.
	pub fn new(min: f64, max: f64, step: f64) -> Result<Self, WatchError> {
		if !min.is_finite() || !max.is_finite() || !step.is_finite() || min >= max || step <= 0.0 {
			return Err(WatchError::InvalidState(
				"crown config requires finite min < max and step > 0".into(),
			));
		}
		Ok(Self {
			min,
			max,
			step,
			sensitivity: CrownSensitivity::Medium,
			haptic_detents: true,
			wrap: false,
		})
	}

	/// Sets rotary sensitivity.
	#[must_use]
	pub const fn sensitivity(mut self, sensitivity: CrownSensitivity) -> Self {
		self.sensitivity = sensitivity;
		self
	}

	/// Enables or disables detent haptic ticks.
	#[must_use]
	pub const fn haptic_detents(mut self, enabled: bool) -> Self {
		self.haptic_detents = enabled;
		self
	}

	/// Enables wrap-around for circular dials.
	#[must_use]
	pub const fn wrap(mut self, wrap: bool) -> Self {
		self.wrap = wrap;
		self
	}
}

/// One rotary input sample from the Digital Crown or Wear OS bezel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrownEvent {
	/// Signed rotation delta in config units.
	pub delta: f64,
	/// Angular velocity (units per second).
	pub velocity: f64,
	/// `true` once the crown/bezel settles idle.
	pub idle: bool,
}

/// Stateful accumulator applying [`CrownEvent`]s against a [`CrownConfig`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotaryState {
	config: CrownConfig,
	value: f64,
}

impl RotaryState {
	/// Starts rotary tracking at `initial` (clamped into `[min, max]`,
	/// falling back to `min` when `initial` is non-finite).
	#[must_use]
	pub const fn new(config: CrownConfig, initial: f64) -> Self {
		let clamped = if initial.is_finite() {
			initial.clamp(config.min, config.max)
		} else {
			config.min
		};
		Self {
			config,
			value: clamped,
		}
	}

	/// Active configuration.
	#[must_use]
	pub const fn config(&self) -> CrownConfig {
		self.config
	}

	/// Current accumulated value.
	#[must_use]
	pub const fn value(&self) -> f64 {
		self.value
	}

	/// Applies `event` and returns how many discrete `step` detent ticks were
	/// crossed (for triggering watch haptics).
	pub fn apply(&mut self, event: CrownEvent) -> u32 {
		if !event.delta.is_finite() {
			return 0;
		}
		let previous_bucket = ((self.value - self.config.min) / self.config.step).floor();
		let scale = match self.config.sensitivity {
			CrownSensitivity::Low => 0.5,
			CrownSensitivity::Medium => 1.0,
			CrownSensitivity::High => 2.0,
		};
		let span = self.config.max - self.config.min;
		let next = self.value + (event.delta * scale);
		let unwrapped_target = if self.config.wrap {
			next
		} else {
			next.clamp(self.config.min, self.config.max)
		};
		let next_bucket = ((unwrapped_target - self.config.min) / self.config.step).floor();
		self.value = if self.config.wrap {
			let offset = (next - self.config.min).rem_euclid(span);
			self.config.min + offset
		} else {
			unwrapped_target
		};
		#[allow(
			clippy::cast_possible_truncation,
			clippy::cast_sign_loss,
			reason = "detent count is bounded and nonnegative"
		)]
		{
			(next_bucket - previous_bucket).abs() as u32
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rotary_state_clamps_wraps_and_counts_detents() {
		let config = CrownConfig::new(0.0, 100.0, 10.0).unwrap();
		let mut state = RotaryState::new(config, 50.0);
		let detents = state.apply(CrownEvent {
			delta: 25.0,
			velocity: 5.0,
			idle: false,
		});
		assert!((state.value() - 75.0).abs() < f64::EPSILON);
		assert_eq!(detents, 2);

		let _ = state.apply(CrownEvent {
			delta: 100.0,
			velocity: 10.0,
			idle: true,
		});
		assert!((state.value() - 100.0).abs() < f64::EPSILON);

		let wrap_cfg = CrownConfig::new(0.0, 360.0, 30.0).unwrap().wrap(true);
		let mut dial = RotaryState::new(wrap_cfg, 350.0);
		let wrapped_detents = dial.apply(CrownEvent {
			delta: 20.0,
			velocity: 1.0,
			idle: false,
		});
		assert!((dial.value() - 10.0).abs() < 1e-9);
		assert_eq!(wrapped_detents, 1);

		let nan_state = RotaryState::new(config, f64::NAN);
		assert!((nan_state.value() - 0.0).abs() < f64::EPSILON);
	}
}
