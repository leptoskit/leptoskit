//! Sample-by-sample building blocks for the mixer: gain ramps, a smoothed
//! resonant filter and a cubic resampler. Nothing here allocates.

use std::f32::consts::PI;

use crate::Filter;
use crate::FilterKind;

/// The shortest change of anything audible, so that no change clicks.
pub const MIN_RAMP_SECONDS: f32 = 0.005;

/// The resonance of a filter with no peak at its cutoff (Q of 1/√2).
const PLAIN: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// The highest cutoff, as a fraction of the sample rate.
pub const CEILING: f32 = 0.45;

/// A value that moves linearly to a target over a number of frames.
#[derive(Debug, Clone, Copy)]
pub struct Ramp {
	value: f32,
	target: f32,
	step: f32,
	remaining: u32,
}

impl Ramp {
	pub fn new(value: f32) -> Self {
		Self {
			value,
			target: value,
			step: 0.0,
			remaining: 0,
		}
	}

	/// Moves to `target` over `frames`; zero frames jumps there.
	pub fn set(&mut self, target: f32, frames: u32) {
		self.target = target;

		if frames == 0 {
			self.value = target;
			self.remaining = 0;
		} else {
			self.step = (target - self.value) / frames as f32;
			self.remaining = frames;
		}
	}

	/// Advances one frame and returns the new value.
	#[inline]
	pub fn next(&mut self) -> f32 {
		if self.remaining > 0 {
			self.remaining -= 1;
			// Measured back from the target, so rounding never accumulates.
			self.value = self.target - self.step * self.remaining as f32;
		}

		self.value
	}

	pub fn value(&self) -> f32 {
		self.value
	}

	pub fn settled(&self) -> bool {
		self.remaining == 0
	}
}

/// Frames in `seconds` at `rate`, never fewer than the click-free minimum.
pub fn frames(seconds: f32, rate: f32) -> u32 {
	(seconds.max(MIN_RAMP_SECONDS) * rate).round() as u32
}

/// A two-pole state-variable filter, trapezoidal-integrated (Simper's SVF).
/// Its response is a biquad's, but it stays stable and quiet while its
/// coefficients change every sample, which is what lets the cutoff glide
/// without zipper noise.
#[derive(Debug, Clone, Copy)]
struct Svf {
	a1: f32,
	a2: f32,
	a3: f32,
	k: f32,
	state: [[f32; 2]; 2],
}

impl Svf {
	fn new() -> Self {
		Self {
			a1: 1.0,
			a2: 0.0,
			a3: 0.0,
			k: 1.0,
			state: [[0.0; 2]; 2],
		}
	}

	fn tune(&mut self, cutoff: f32, resonance: f32, rate: f32) {
		let g = (PI * cutoff / rate).tan();
		self.k = 1.0 / resonance;
		self.a1 = 1.0 / (1.0 + g * (g + self.k));
		self.a2 = g * self.a1;
		self.a3 = g * self.a2;
	}

	#[inline]
	fn process(&mut self, kind: FilterKind, input: [f32; 2]) -> [f32; 2] {
		let mut output = [0.0; 2];

		for (channel, sample) in input.into_iter().enumerate() {
			let [ic1, ic2] = &mut self.state[channel];
			let v3 = sample - *ic2;
			let v1 = self.a1 * *ic1 + self.a2 * v3;
			let v2 = *ic2 + self.a2 * *ic1 + self.a3 * v3;
			*ic1 = 2.0 * v1 - *ic1;
			*ic2 = 2.0 * v2 - *ic2;

			output[channel] = match kind {
				FilterKind::LowPass => v2,
				FilterKind::HighPass => sample - self.k * v1 - v2,
			};
		}

		output
	}
}

/// One filter slot. The cutoff glides in octaves (its logarithm moves
/// linearly), and the resonance glides with it. A slot that is switched on
/// starts fully open and closes to its cutoff; one switched off opens fully
/// and then stops, so neither clicks.
#[derive(Debug, Clone, Copy)]
pub struct FilterSlot {
	kind: Option<FilterKind>,
	closing: bool,
	log_cutoff: Ramp,
	resonance: Ramp,
	svf: Svf,
	rate: f32,
}

impl FilterSlot {
	pub fn new(rate: f32) -> Self {
		Self {
			kind: None,
			closing: false,
			log_cutoff: Ramp::new(0.0),
			resonance: Ramp::new(PLAIN),
			svf: Svf::new(),
			rate,
		}
	}

	/// The cutoff at which a filter of `kind` does nothing.
	fn open(&self, kind: FilterKind) -> f32 {
		match kind {
			FilterKind::LowPass => self.ceiling(),
			FilterKind::HighPass => crate::MIN_CUTOFF,
		}
	}

	/// Cutoffs stay below 45% of the rate, where the filter is still sound.
	fn ceiling(&self) -> f32 {
		self.rate * CEILING
	}

	/// Glides to `filter`, or open and then off for `None`.
	pub fn set(&mut self, filter: Option<Filter>, frames: u32) {
		match (filter, self.kind) {
			(Some(filter), _) => self.glide_to(filter, frames),
			(None, Some(kind)) => {
				self.closing = true;
				self.log_cutoff.set(self.open(kind).ln(), frames);
				self.resonance.set(PLAIN, frames);
			}
			(None, None) => {}
		}
	}

	fn glide_to(&mut self, filter: Filter, frames: u32) {
		// A new shape starts open, where it changes nothing.
		if self.kind != Some(filter.kind) {
			self.kind = Some(filter.kind);
			self.svf = Svf::new();
			self.log_cutoff = Ramp::new(self.open(filter.kind).ln());
			self.resonance = Ramp::new(PLAIN);
		}

		self.closing = false;
		let cutoff = filter.cutoff.clamp(crate::MIN_CUTOFF, self.ceiling());
		self.log_cutoff.set(cutoff.ln(), frames);
		self.resonance.set(filter.resonance, frames);
		self.retune();
	}

	fn retune(&mut self) {
		self.svf.tune(
			self.log_cutoff.value().exp(),
			self.resonance.value(),
			self.rate,
		);
	}

	#[cfg(test)]
	pub fn active(&self) -> bool {
		self.kind.is_some()
	}

	/// The cutoff now, in hertz, while a filter is on.
	#[cfg(test)]
	pub fn cutoff(&self) -> Option<f32> {
		self.kind.map(|_| self.log_cutoff.value().exp())
	}

	#[inline]
	pub fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
		let Some(kind) = self.kind else {
			return input;
		};

		if !(self.log_cutoff.settled() && self.resonance.settled()) {
			self.log_cutoff.next();
			self.resonance.next();
			self.retune();
		} else if self.closing {
			self.kind = None;
			self.closing = false;

			return input;
		}

		self.svf.process(kind, input)
	}
}

/// Turns a stream of frames at one rate into frames at another, with
/// four-point cubic (Hermite) interpolation. At equal rates it passes frames
/// through unchanged, two frames late.
#[derive(Debug, Clone, Copy)]
pub struct Resampler {
	step: f64,
	phase: f64,
	history: [[f32; 2]; 4],
}

impl Resampler {
	pub fn new(source_rate: u32, output_rate: u32) -> Self {
		Self {
			step: f64::from(source_rate) / f64::from(output_rate.max(1)),
			phase: 1.0,
			history: [[0.0; 2]; 4],
		}
	}

	/// How many source frames the next output frame needs.
	#[inline]
	pub fn wanted(&self) -> u32 {
		self.phase as u32
	}

	/// Shifts one source frame in.
	#[inline]
	pub fn push(&mut self, frame: [f32; 2]) {
		self.history.copy_within(1.., 0);
		self.history[3] = frame;
		self.phase -= 1.0;
	}

	/// The next output frame; call once [`Self::wanted`] frames are pushed.
	#[inline]
	pub fn next(&mut self) -> [f32; 2] {
		let x = self.phase as f32;
		let [y0, y1, y2, y3] = self.history;
		let mut output = [0.0; 2];

		for channel in 0..2 {
			let (y0, y1, y2, y3) = (y0[channel], y1[channel], y2[channel], y3[channel]);
			let c1 = 0.5 * (y2 - y0);
			let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
			let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
			output[channel] = ((c3 * x + c2) * x + c1) * x + y1;
		}

		self.phase += self.step;
		output
	}
}

#[cfg(test)]
pub(crate) mod tests;
