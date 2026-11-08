//! Ramps, filters and resampling, measured on rendered signals.

use super::*;

pub fn sine(frequency: f32, rate: f32, frames: usize) -> Vec<f32> {
	(0..frames)
		.map(|frame| (2.0 * PI * frequency * frame as f32 / rate).sin())
		.collect()
}

pub fn rms(samples: &[f32]) -> f32 {
	(samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

fn filtered(filter: Filter, signal: &[f32], rate: f32) -> Vec<f32> {
	let mut slot = FilterSlot::new(rate);
	slot.set(Some(filter), 0);
	signal
		.iter()
		.map(|&sample| slot.process([sample, sample])[0])
		.collect()
}

#[test]
fn a_low_pass_at_500_hz_takes_a_5_khz_sine_down_and_lets_100_hz_through() {
	let rate = 48_000.0;
	let veil = Filter::low_pass(500.0);
	// Skip the first tenth of a second, while the filter settles.
	let high = filtered(veil, &sine(5_000.0, rate, 48_000), rate);
	let low = filtered(veil, &sine(100.0, rate, 48_000), rate);
	let high = rms(&high[4_800..]) / rms(&sine(5_000.0, rate, 43_200));
	let low = rms(&low[4_800..]) / rms(&sine(100.0, rate, 43_200));

	// Two poles fall 12 dB an octave: a decade above, about -40 dB.
	assert!(high < 0.012, "5 kHz kept {high}");
	assert!(low > 0.97, "100 Hz kept only {low}");
}

#[test]
fn a_high_pass_does_the_opposite() {
	let rate = 44_100.0;
	let thin = Filter::high_pass(2_000.0);
	let low = filtered(thin, &sine(200.0, rate, 44_100), rate);
	let high = filtered(thin, &sine(10_000.0, rate, 44_100), rate);

	assert!(rms(&low[4_410..]) < 0.02);
	assert!(rms(&high[4_410..]) > 0.68);
}

#[test]
fn resonance_peaks_at_the_cutoff() {
	let rate = 48_000.0;
	let flat = filtered(
		Filter::low_pass(1_000.0),
		&sine(1_000.0, rate, 24_000),
		rate,
	);
	let ringing = filtered(
		Filter::low_pass(1_000.0).resonance(8.0),
		&sine(1_000.0, rate, 24_000),
		rate,
	);

	assert!(rms(&ringing[12_000..]) > 5.0 * rms(&flat[12_000..]));
}

/// The harshest case for a moving filter: a full-scale square wave while the
/// cutoff sweeps down five octaves and the resonance rises, all at once.
#[test]
fn a_gliding_cutoff_never_jumps() {
	let rate = 48_000.0;
	let mut slot = FilterSlot::new(rate);
	slot.set(Some(Filter::low_pass(8_000.0)), 0);
	// A full-scale square wave is the harshest input for a moving filter.
	let input: Vec<f32> = (0..48_000)
		.map(|frame| if (frame / 48) % 2 == 0 { 1.0 } else { -1.0 })
		.collect();
	let mut output = Vec::with_capacity(input.len());

	for (index, &sample) in input.iter().enumerate() {
		if index == 1_000 {
			slot.set(
				Some(Filter::low_pass(300.0).resonance(4.0)),
				frames(0.5, rate),
			);
		}

		output.push(slot.process([sample, sample])[0]);
	}

	let cutoff = slot.cutoff().unwrap();
	assert!((cutoff - 300.0).abs() < 0.5, "{cutoff}");
	assert!(
		output
			.iter()
			.all(|sample| sample.is_finite() && sample.abs() < 6.0)
	);
}

#[test]
fn switching_a_filter_on_and_off_glides_through_open() {
	let rate = 48_000.0;
	let mut slot = FilterSlot::new(rate);
	assert!(!slot.active());
	slot.set(Some(Filter::low_pass(1_000.0)), 480);
	assert!((slot.cutoff().unwrap() - rate * 0.45).abs() < 1.0);

	for _ in 0..480 {
		slot.process([0.0; 2]);
	}

	assert!((slot.cutoff().unwrap() - 1_000.0).abs() < 0.1);
	slot.set(None, 480);

	for _ in 0..481 {
		slot.process([0.0; 2]);
	}

	assert!(!slot.active());
	assert_eq!(slot.process([0.25, -0.5]), [0.25, -0.5]);
}

#[test]
fn ramps_move_in_equal_steps_and_land_exactly() {
	let mut ramp = Ramp::new(0.0);
	ramp.set(1.0, 4);
	let steps: Vec<f32> = (0..6).map(|_| ramp.next()).collect();
	assert_eq!(steps, [0.25, 0.5, 0.75, 1.0, 1.0, 1.0]);
	assert!(ramp.settled());

	// A change mid-ramp starts from where the ramp is, never jumping.
	ramp.set(0.0, 1_000);

	for _ in 0..500 {
		ramp.next();
	}

	ramp.set(1.0, 10);
	let before = ramp.value();
	let after = ramp.next();
	assert!((after - before).abs() < 0.06);
	assert_eq!(frames(0.0, 48_000.0), 240);
}

#[test]
fn equal_rates_pass_frames_through_two_frames_late() {
	let mut resampler = Resampler::new(48_000, 48_000);
	let input: Vec<[f32; 2]> = (0..16)
		.map(|frame| [frame as f32, -(frame as f32)])
		.collect();
	let mut frames = input.iter();
	let mut output = Vec::new();

	for _ in 0..16 {
		while resampler.wanted() > 0 {
			resampler.push(*frames.next().unwrap_or(&[0.0; 2]));
		}

		output.push(resampler.next());
	}

	assert_eq!(output[..2], [[0.0, 0.0]; 2]);
	assert_eq!(output[2..], input[..14]);
}

#[test]
fn resampling_keeps_a_tone_s_pitch_and_level() {
	// A 1 kHz tone recorded at 44.1 kHz, played out at 48 kHz.
	let source = sine(1_000.0, 44_100.0, 44_100);
	let mut resampler = Resampler::new(44_100, 48_000);
	let mut samples = source.iter();
	let output: Vec<f32> = (0..48_000)
		.map(|_| {
			while resampler.wanted() > 0 {
				let sample = *samples.next().unwrap_or(&0.0);
				resampler.push([sample, sample]);
			}

			resampler.next()[0]
		})
		.collect();
	let expected = sine(1_000.0, 48_000.0, 48_000);
	// Two source frames of latency at 44.1 kHz.
	let delay = 2.0 * 48_000.0 / 44_100.0;
	let error = output[100..47_000]
		.iter()
		.enumerate()
		.map(|(index, sample)| {
			let time = (index + 100) as f32 - delay;
			let wanted = (2.0 * PI * 1_000.0 * time / 48_000.0).sin();
			(sample - wanted).abs()
		})
		.fold(0.0_f32, f32::max);

	assert!(error < 0.01, "worst error {error}");
	assert!((rms(&output[100..47_000]) - rms(&expected)).abs() < 0.01);
}
