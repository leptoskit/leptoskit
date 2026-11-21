//! Live watch workout / exercise sessions (`HKWorkoutSession` on watchOS and
//! Health Services `ExerciseClient` on Wear OS).

use crate::WatchError;

/// Activity type tracked by a watch workout session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkoutActivity {
	/// Outdoor or treadmill running.
	Running,
	/// Outdoor or indoor walking.
	Walking,
	/// Road or stationary cycling.
	Cycling,
	/// Pool or open-water swimming.
	Swimming,
	/// Trail hiking.
	Hiking,
	/// Weight / strength training.
	StrengthTraining,
	/// Yoga / mobility.
	Yoga,
	/// High-intensity interval training.
	Hiit,
	/// Rowing.
	Rowing,
	/// General workout.
	Other,
}

/// Indoor vs. outdoor location hint (controls GPS activation on watchOS and
/// Wear OS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkoutLocation {
	/// Indoor workout (GPS disabled).
	Indoor,
	/// Outdoor workout (GPS enabled when permitted).
	Outdoor,
}

/// Lifecycle state of a [`WorkoutSession`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkoutState {
	/// Configured and ready to begin collecting biometrics.
	Prepared,
	/// Actively recording workout telemetry.
	Running,
	/// Temporarily paused.
	Paused,
	/// Completed and finalized.
	Ended,
}

/// Live biometric telemetry snapshot delivered during a workout.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WorkoutMetrics {
	/// Active elapsed seconds.
	pub elapsed_secs: f64,
	/// Latest heart rate in beats per minute.
	pub heart_rate_bpm: Option<f32>,
	/// Cumulative active energy burned in kilocalories.
	pub active_calories_kcal: Option<f64>,
	/// Cumulative distance in meters.
	pub distance_meters: Option<f64>,
	/// Latest step / pedal cadence per minute.
	pub cadence_spm: Option<f32>,
	/// Cumulative elevation gain in meters.
	pub elevation_gain_meters: Option<f64>,
}

/// Validated state machine for a live watch workout session.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkoutSession {
	activity: WorkoutActivity,
	location: WorkoutLocation,
	state: WorkoutState,
	metrics: WorkoutMetrics,
}

impl WorkoutSession {
	/// Prepares a new workout session without starting sensors yet.
	#[must_use]
	pub fn prepare(activity: WorkoutActivity, location: WorkoutLocation) -> Self {
		Self {
			activity,
			location,
			state: WorkoutState::Prepared,
			metrics: WorkoutMetrics::default(),
		}
	}

	/// Activity type.
	#[must_use]
	pub const fn activity(&self) -> WorkoutActivity {
		self.activity
	}

	/// Indoor or outdoor location mode.
	#[must_use]
	pub const fn location(&self) -> WorkoutLocation {
		self.location
	}

	/// Latest telemetry snapshot.
	#[must_use]
	pub const fn metrics(&self) -> WorkoutMetrics {
		self.metrics
	}

	/// Current lifecycle state.
	#[must_use]
	pub const fn state(&self) -> WorkoutState {
		self.state
	}

	/// Transitions `Prepared -> Running`.
	///
	/// # Errors
	///
	/// Returns [`WatchError::InvalidState`] if the session is not `Prepared`.
	pub fn start(&mut self) -> Result<(), WatchError> {
		if self.state != WorkoutState::Prepared {
			return Err(WatchError::InvalidState(
				"workout can only start from Prepared state".into(),
			));
		}
		self.state = WorkoutState::Running;
		Ok(())
	}

	/// Transitions `Running -> Paused`.
	///
	/// # Errors
	///
	/// Returns [`WatchError::InvalidState`] if the session is not `Running`.
	pub fn pause(&mut self) -> Result<(), WatchError> {
		if self.state != WorkoutState::Running {
			return Err(WatchError::InvalidState(
				"workout can only pause while Running".into(),
			));
		}
		self.state = WorkoutState::Paused;
		Ok(())
	}

	/// Transitions `Paused -> Running`.
	///
	/// # Errors
	///
	/// Returns [`WatchError::InvalidState`] if the session is not `Paused`.
	pub fn resume(&mut self) -> Result<(), WatchError> {
		if self.state != WorkoutState::Paused {
			return Err(WatchError::InvalidState(
				"workout can only resume while Paused".into(),
			));
		}
		self.state = WorkoutState::Running;
		Ok(())
	}

	/// Transitions `Prepared | Running | Paused -> Ended`.
	///
	/// # Errors
	///
	/// Returns [`WatchError::InvalidState`] if already `Ended`.
	pub fn end(&mut self) -> Result<(), WatchError> {
		if self.state == WorkoutState::Ended {
			return Err(WatchError::InvalidState("workout already ended".into()));
		}
		self.state = WorkoutState::Ended;
		Ok(())
	}

	/// Records a live biometric sample while `Running`.
	///
	/// # Errors
	///
	/// Returns [`WatchError::InvalidState`] if the session is not `Running` or
	/// `elapsed_secs` is negative/nonfinite.
	pub fn record_metrics(&mut self, metrics: WorkoutMetrics) -> Result<(), WatchError> {
		if self.state != WorkoutState::Running {
			return Err(WatchError::InvalidState(
				"metrics can only be recorded while Running".into(),
			));
		}
		if !metrics.elapsed_secs.is_finite() || metrics.elapsed_secs < 0.0 {
			return Err(WatchError::InvalidState(
				"elapsed_secs must be finite and nonnegative".into(),
			));
		}
		self.metrics = metrics;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn workout_session_enforces_lifecycle_transitions() {
		let mut session =
			WorkoutSession::prepare(WorkoutActivity::Running, WorkoutLocation::Outdoor);
		assert_eq!(session.state(), WorkoutState::Prepared);
		assert!(session.pause().is_err());

		session.start().unwrap();
		session
			.record_metrics(WorkoutMetrics {
				elapsed_secs: 60.0,
				heart_rate_bpm: Some(152.0),
				active_calories_kcal: Some(14.5),
				distance_meters: Some(210.0),
				cadence_spm: Some(174.0),
				elevation_gain_meters: Some(4.0),
			})
			.unwrap();
		assert_eq!(session.metrics().heart_rate_bpm, Some(152.0));

		session.pause().unwrap();
		assert!(session.record_metrics(WorkoutMetrics::default()).is_err());
		session.resume().unwrap();
		session.end().unwrap();
		assert!(session.end().is_err());
	}
}
