//! Sampling clips and blending them into a pose.

use glam::Quat;
use glam::Vec3;

use crate::asset::Asset;
use crate::asset::Channel;
use crate::asset::ChannelPath;
use crate::asset::Interpolation;
use crate::protocol::LoopMode;

/// Every asset node's local transform and morph weights.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pose {
	pub translation: Vec<Vec3>,
	pub rotation: Vec<Quat>,
	pub scale: Vec<Vec3>,
	/// Per node; empty for nodes without a mesh with targets.
	pub weights: Vec<Vec<f32>>,
}

impl Pose {
	/// The pose the file was authored in.
	pub fn rest(asset: &Asset) -> Self {
		Self {
			translation: asset.nodes.iter().map(|node| node.translation).collect(),
			rotation: asset.nodes.iter().map(|node| node.rotation).collect(),
			scale: asset.nodes.iter().map(|node| node.scale).collect(),
			weights: asset
				.nodes
				.iter()
				.map(|node| {
					node.mesh
						.map(|mesh| {
							let defaults = &asset.meshes[mesh].weights;
							let mut weights =
								node.weights.clone().unwrap_or_else(|| defaults.clone());
							weights.resize(defaults.len(), 0.0);
							weights
						})
						.unwrap_or_default()
				})
				.collect(),
		}
	}

	fn copy_from(&mut self, other: &Self) {
		self.translation.copy_from_slice(&other.translation);
		self.rotation.copy_from_slice(&other.rotation);
		self.scale.copy_from_slice(&other.scale);
		for (weights, source) in self.weights.iter_mut().zip(&other.weights) {
			weights.copy_from_slice(source);
		}
	}
}

/// One clip playing.
#[derive(Clone, Debug, PartialEq)]
struct Track {
	clip: usize,
	time: f32,
	speed: f32,
	mode: LoopMode,
	/// How much this track shows over the ones before it.
	weight: f32,
	/// Change in weight per second while fading in.
	fade_rate: f32,
	finished: bool,
}

/// The clips playing on one instance, and the crossfades between them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Animator {
	/// Oldest first. The newest fades in over the others; once it shows fully,
	/// the others are dropped.
	tracks: Vec<Track>,
	/// Fades every track out towards the rest pose after a stop.
	master: f32,
	master_rate: f32,
}

/// What advancing an animator did.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Advance {
	/// Clips played once that reached their end during this step.
	pub finished: Vec<usize>,
}

/// Where a clip's time lands for sampling.
fn local_time(time: f32, duration: f32, mode: LoopMode) -> f32 {
	if duration <= 0.0 {
		return 0.0;
	}
	match mode {
		LoopMode::Once => time.clamp(0.0, duration),
		LoopMode::Loop => time.rem_euclid(duration),
		LoopMode::PingPong => {
			let phase = time.rem_euclid(duration * 2.0);
			duration - (phase - duration).abs()
		}
	}
}

impl Animator {
	/// Whether any clip is playing or fading.
	pub fn is_active(&self) -> bool {
		!self.tracks.is_empty()
	}

	/// Whether the pose would change with time.
	pub fn is_moving(&self) -> bool {
		self.master_rate != 0.0
			|| self
				.tracks
				.iter()
				.any(|track| track.fade_rate != 0.0 || (!track.finished && track.speed != 0.0))
	}

	pub fn play(&mut self, clip: usize, speed: f32, mode: LoopMode, fade: f32, duration: f32) {
		let start = if speed < 0.0 && mode == LoopMode::Once {
			duration
		} else {
			0.0
		};
		if fade <= 0.0 || self.tracks.is_empty() || self.master <= 0.0 {
			self.tracks.clear();
		}
		let fading = !self.tracks.is_empty();
		// A stop in progress is cancelled: what is still showing fades into
		// the new clip.
		self.master = 1.0;
		self.master_rate = 0.0;
		self.tracks.push(Track {
			clip,
			time: start,
			speed,
			mode,
			weight: if fading { 0.0 } else { 1.0 },
			fade_rate: if fading { 1.0 / fade } else { 0.0 },
			finished: false,
		});
		// Bound the tracks a guest can stack by crossfading repeatedly.
		if self.tracks.len() > 4 {
			self.tracks.drain(..self.tracks.len() - 4);
		}
	}

	pub fn stop(&mut self, fade: f32) {
		if fade <= 0.0 {
			self.tracks.clear();
			self.master_rate = 0.0;
		} else if !self.tracks.is_empty() {
			self.master_rate = -1.0 / fade;
		}
	}

	/// Moves time forward by `elapsed` seconds.
	pub fn advance(&mut self, asset: &Asset, elapsed: f32) -> Advance {
		let mut advance = Advance::default();
		for track in &mut self.tracks {
			let duration = asset.clips[track.clip].duration;
			if !track.finished {
				track.time += elapsed * track.speed;
				if track.mode == LoopMode::Once
					&& ((track.speed > 0.0 && track.time >= duration)
						|| (track.speed < 0.0 && track.time <= 0.0))
				{
					track.time = track.time.clamp(0.0, duration);
					track.finished = true;
					advance.finished.push(track.clip);
				}
				// Keep looping time small so it never loses precision.
				if track.mode != LoopMode::Once && duration > 0.0 {
					track.time = track.time.rem_euclid(duration * 2.0);
				}
			}
			if track.fade_rate != 0.0 {
				track.weight = (track.weight + track.fade_rate * elapsed).min(1.0);
				if track.weight >= 1.0 {
					track.fade_rate = 0.0;
				}
			}
		}
		if let Some(position) = self.tracks.iter().rposition(|track| track.weight >= 1.0) {
			self.tracks.drain(..position);
		}
		if self.master_rate != 0.0 {
			self.master += self.master_rate * elapsed;
			if self.master <= 0.0 {
				self.tracks.clear();
				self.master = 1.0;
				self.master_rate = 0.0;
			}
		}
		advance
	}

	/// Writes the blended pose into `pose`, starting from `rest`.
	pub fn sample(&self, asset: &Asset, rest: &Pose, pose: &mut Pose, touched: &mut Vec<usize>) {
		pose.copy_from(rest);
		touched.clear();
		for (index, track) in self.tracks.iter().enumerate() {
			let clip = &asset.clips[track.clip];
			let time = local_time(track.time, clip.duration, track.mode);
			let weight = if index == 0 { 1.0 } else { track.weight };
			for channel in &clip.channels {
				apply_channel(channel, time, weight, pose);
				touched.push(channel.node);
			}
		}
		if self.master < 1.0 {
			let master = self.master.max(0.0);
			touched.sort_unstable();
			touched.dedup();
			for &node in touched.iter() {
				pose.translation[node] =
					rest.translation[node].lerp(pose.translation[node], master);
				pose.rotation[node] = rest.rotation[node].slerp(pose.rotation[node], master);
				pose.scale[node] = rest.scale[node].lerp(pose.scale[node], master);
				for (value, rest) in pose.weights[node].iter_mut().zip(&rest.weights[node]) {
					*value = rest + (*value - rest) * master;
				}
			}
		}
	}
}

/// The keys around `time` and how far between them it is.
fn keys(times: &[f32], time: f32) -> (usize, usize, f32, f32) {
	let last = times.len() - 1;
	if time <= times[0] {
		return (0, 0, 0.0, 0.0);
	}
	if time >= times[last] {
		return (last, last, 0.0, 0.0);
	}
	let next = times.partition_point(|key| *key <= time).min(last);
	let previous = next - 1;
	let span = times[next] - times[previous];
	let t = if span > 0.0 {
		(time - times[previous]) / span
	} else {
		0.0
	};
	(previous, next, t, span)
}

/// Samples `width` values from a channel at `time` into `output`.
fn sample(channel: &Channel, time: f32, output: &mut [f32]) {
	let width = channel.width;
	if channel.times.is_empty() {
		return;
	}
	let (previous, next, t, span) = keys(&channel.times, time);
	let values = &channel.values;
	match channel.interpolation {
		Interpolation::Step => {
			output.copy_from_slice(&values[previous * width..previous * width + width]);
		}
		Interpolation::Linear => {
			let a = &values[previous * width..previous * width + width];
			let b = &values[next * width..next * width + width];
			for (slot, (a, b)) in output.iter_mut().zip(a.iter().zip(b)) {
				*slot = a + (b - a) * t;
			}
		}
		Interpolation::CubicSpline => {
			// Each key holds an in tangent, a value, and an out tangent.
			let key = |index: usize, part: usize| {
				&values[(index * 3 + part) * width..(index * 3 + part + 1) * width]
			};
			let (p0, m0) = (key(previous, 1), key(previous, 2));
			let (p1, m1) = (key(next, 1), key(next, 0));
			let (t2, t3) = (t * t, t * t * t);
			let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
			let h10 = t3 - 2.0 * t2 + t;
			let h01 = -2.0 * t3 + 3.0 * t2;
			let h11 = t3 - t2;
			for index in 0..width {
				output[index] = h00 * p0[index]
					+ h10 * span * m0[index]
					+ h01 * p1[index]
					+ h11 * span * m1[index];
			}
		}
	}
}

fn apply_channel(channel: &Channel, time: f32, weight: f32, pose: &mut Pose) {
	let node = channel.node;
	match channel.path {
		ChannelPath::Translation | ChannelPath::Scale => {
			let mut value = [0.0; 3];
			sample(channel, time, &mut value);
			let value = Vec3::from(value);
			let slot = if channel.path == ChannelPath::Translation {
				&mut pose.translation[node]
			} else {
				&mut pose.scale[node]
			};
			*slot = if weight >= 1.0 {
				value
			} else {
				slot.lerp(value, weight)
			};
		}
		ChannelPath::Rotation => {
			let rotation = if channel.interpolation == Interpolation::Linear {
				let (previous, next, t, _) = keys(&channel.times, time);
				let at = |index: usize| Quat::from_slice(&channel.values[index * 4..index * 4 + 4]);
				at(previous).slerp(at(next), t)
			} else {
				let mut value = [0.0; 4];
				sample(channel, time, &mut value);
				Quat::from_array(value)
			};
			let rotation = rotation.try_normalize().unwrap_or(Quat::IDENTITY);
			let slot = &mut pose.rotation[node];
			*slot = if weight >= 1.0 {
				rotation
			} else {
				slot.slerp(rotation, weight)
			};
		}
		ChannelPath::Weights => {
			let weights = &mut pose.weights[node];
			if weights.len() != channel.width {
				return;
			}
			let mut value = [0.0; crate::sanitize::MAX_MORPH_TARGETS];
			let value = &mut value[..channel.width.min(crate::sanitize::MAX_MORPH_TARGETS)];
			sample(channel, time, value);
			for (slot, value) in weights.iter_mut().zip(value.iter()) {
				*slot += (value - *slot) * weight.min(1.0);
			}
		}
	}
}

trait QuatExt {
	fn try_normalize(self) -> Option<Quat>;
}

impl QuatExt for Quat {
	fn try_normalize(self) -> Option<Quat> {
		let length = self.length();
		(length > 1.0e-12 && length.is_finite()).then(|| self / length)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn loop_modes_map_time_into_the_clip() {
		assert_eq!(local_time(2.5, 2.0, LoopMode::Loop), 0.5);
		assert_eq!(local_time(-0.5, 2.0, LoopMode::Loop), 1.5);
		assert_eq!(local_time(5.0, 2.0, LoopMode::Once), 2.0);
		assert_eq!(local_time(2.5, 2.0, LoopMode::PingPong), 1.5);
		assert_eq!(local_time(3.5, 2.0, LoopMode::PingPong), 0.5);
		assert_eq!(local_time(1.0, 0.0, LoopMode::Loop), 0.0);
	}

	#[test]
	fn keys_clamp_outside_the_range_and_interpolate_inside_it() {
		let times = [0.0, 1.0, 3.0];
		assert_eq!(keys(&times, -1.0), (0, 0, 0.0, 0.0));
		assert_eq!(keys(&times, 9.0), (2, 2, 0.0, 0.0));
		assert_eq!(keys(&times, 2.0), (1, 2, 0.5, 2.0));
		assert_eq!(keys(&times, 1.0), (1, 2, 0.0, 2.0));
	}

	fn channel(interpolation: Interpolation, values: Vec<f32>) -> Channel {
		Channel {
			node: 0,
			path: ChannelPath::Translation,
			interpolation,
			times: vec![0.0, 1.0],
			values,
			width: 1,
		}
	}

	#[test]
	fn step_linear_and_cubic_sampling() {
		let mut out = [0.0];
		sample(
			&channel(Interpolation::Step, vec![1.0, 3.0]),
			0.75,
			&mut out,
		);
		assert_eq!(out, [1.0]);
		sample(
			&channel(Interpolation::Linear, vec![1.0, 3.0]),
			0.75,
			&mut out,
		);
		assert_eq!(out, [2.5]);
		// Zero tangents: a smoothstep from 1 to 3.
		let cubic = channel(
			Interpolation::CubicSpline,
			vec![0.0, 1.0, 0.0, 0.0, 3.0, 0.0],
		);
		sample(&cubic, 0.5, &mut out);
		assert!((out[0] - 2.0).abs() < 1.0e-6);
		sample(&cubic, 1.0, &mut out);
		assert!((out[0] - 3.0).abs() < 1.0e-6);
	}
}
