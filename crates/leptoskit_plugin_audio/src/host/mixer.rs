//! The real-time mixer. It runs inside the device's audio callback, so it
//! never allocates, frees, locks or waits: commands arrive over a lock-free
//! ring buffer, and every voice it is done with goes back over another one to
//! be freed on the control thread.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use rtrb::Consumer;
use rtrb::Producer;
use rtrb::RingBuffer;

use super::dsp::FilterSlot;
use super::dsp::Ramp;
use super::dsp::Resampler;
use super::dsp::frames;
use super::source::Pcm;
use crate::FILTER_SLOTS;
use crate::Filter;

/// Voices the mixer can hold at once.
pub const MAX_VOICES: usize = 64;
/// Commands that can wait for the next callback.
pub const COMMAND_CAPACITY: usize = 256;
/// Frames mixed at a time; callbacks of any size are split into these.
const BLOCK: usize = 256;
/// Silent frames pushed past a sound's end to flush the interpolator.
const FLUSH_FRAMES: u8 = 3;

const SLOTS: usize = FILTER_SLOTS as usize;

/// Where a voice's frames come from.
#[derive(Debug)]
pub enum Feed {
	/// A fully decoded sound, shared by every voice of it.
	Memory { pcm: Arc<Pcm>, frame: usize },
	/// Frames a decoder thread keeps a second ahead of the voice.
	Stream {
		frames: Consumer<f32>,
		finished: Arc<AtomicBool>,
	},
}

enum Next {
	Frame([f32; 2]),
	/// A stream fell behind: silence, and try again next frame.
	Starved,
	End,
}

impl Feed {
	/// The next frame of the sound: memory sounds wrap round when looping,
	/// and streams loop on their decoder thread.
	#[inline]
	fn next(&mut self, looping: bool) -> Next {
		match self {
			Self::Memory { pcm, frame } => next_in_memory(pcm, frame, looping),
			Self::Stream { frames, finished } => next_streamed(frames, finished),
		}
	}
}

#[inline]
fn next_in_memory(pcm: &Pcm, frame: &mut usize, looping: bool) -> Next {
	if *frame >= pcm.frames() && looping {
		*frame = 0;
	}

	if *frame >= pcm.frames() {
		return Next::End;
	}

	let next = pcm.frame(*frame);
	*frame += 1;
	Next::Frame(next)
}

#[inline]
fn next_streamed(frames: &mut Consumer<f32>, finished: &AtomicBool) -> Next {
	if frames.slots() >= 2 {
		let left = frames.pop().unwrap_or_default();
		let right = frames.pop().unwrap_or_default();
		Next::Frame([left, right])
	} else if finished.load(Ordering::Acquire) {
		Next::End
	} else {
		Next::Starved
	}
}

/// A playing voice, built on the control thread and moved in whole.
#[derive(Debug)]
pub struct VoiceState {
	pub id: u32,
	feed: Feed,
	looping: bool,
	resampler: Resampler,
	/// Frames pushed past the end; three flush the interpolator.
	tail: u8,
	volume: Ramp,
	/// Fade-in, and the fade of a stop.
	fade: Ramp,
	/// Eases pause and resume.
	pause: Ramp,
	paused: bool,
	stopping: bool,
	balance: [f32; 2],
	filters: [FilterSlot; SLOTS],
}

/// How a voice starts.
#[derive(Debug, Clone, Copy)]
pub struct Start {
	pub source_rate: u32,
	pub volume: f32,
	pub pan: f32,
	pub looping: bool,
	pub fade_in: f32,
	pub paused: bool,
}

impl VoiceState {
	pub fn new(id: u32, feed: Feed, start: Start, rate: u32) -> Self {
		let output = rate as f32;
		// A zero fade starts at full level: effects keep their attack.
		let mut fade = Ramp::new(if start.fade_in > 0.0 { 0.0 } else { 1.0 });
		fade.set(1.0, frames(start.fade_in, output));

		let pause = Ramp::new(if start.paused { 0.0 } else { 1.0 });

		Self {
			id,
			feed,
			looping: start.looping,
			resampler: Resampler::new(start.source_rate, rate),
			tail: 0,
			volume: Ramp::new(start.volume),
			fade,
			pause,
			paused: start.paused,
			stopping: false,
			balance: balance(start.pan),
			filters: [FilterSlot::new(output); SLOTS],
		}
	}

	/// Fades out and ends. A paused voice stays silent, but runs again so
	/// that its fade can finish and it can be retired.
	fn stop(&mut self, frames: u32) {
		self.stopping = true;
		self.paused = false;
		self.fade.set(0.0, frames);
	}

	/// Feeds the resampler what the next output frame needs. Returns false
	/// once the end has been flushed through it.
	fn pull(&mut self) -> bool {
		while self.resampler.wanted() > 0 {
			let next = if self.tail > 0 {
				Next::End
			} else {
				self.feed.next(self.looping)
			};

			if matches!(next, Next::End) {
				self.tail += 1;
			}

			let frame = match next {
				Next::Frame(frame) => frame,
				Next::Starved | Next::End => [0.0; 2],
			};

			self.resampler.push(frame);

			if self.tail > FLUSH_FRAMES {
				return false;
			}
		}

		true
	}

	/// The next frame, filtered, at the voice's gain and pan.
	fn next_frame(&mut self) -> [f32; 2] {
		let mut sample = self.resampler.next();

		for filter in &mut self.filters {
			sample = filter.process(sample);
		}

		let gain = self.volume.next() * self.fade.next() * self.pause.next();
		[
			sample[0] * gain * self.balance[0],
			sample[1] * gain * self.balance[1],
		]
	}

	/// Paused and faded all the way out: it holds its place.
	fn resting(&self) -> bool {
		self.paused && self.pause.settled()
	}

	/// Adds up to `output.len() / 2` frames in. Returns false once the voice
	/// has nothing more to give.
	fn render(&mut self, output: &mut [f32]) -> bool {
		for frame in output.as_chunks_mut::<2>().0 {
			if self.resting() {
				return true;
			}

			if !self.pull() {
				return false;
			}

			let [left, right] = self.next_frame();
			frame[0] += left;
			frame[1] += right;

			if self.stopping && self.fade.settled() {
				return false;
			}
		}

		true
	}
}

/// Stereo gains for a pan position: the near side stays at full level.
fn balance(pan: f32) -> [f32; 2] {
	[(1.0 - pan).min(1.0), (1.0 + pan).min(1.0)]
}

/// What a command applies to; voice 0 is the master bus.
pub type Target = u32;

/// A change for the mixer, timed in output frames.
///
/// A voice travels by value: boxing it would free the box on the audio
/// thread.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
	Play(VoiceState),
	Volume {
		target: Target,
		volume: f32,
		frames: u32,
	},
	Stop {
		target: Target,
		frames: u32,
	},
	Pause {
		target: Target,
		paused: bool,
		frames: u32,
	},
	Filter {
		target: Target,
		slot: usize,
		filter: Option<Filter>,
		frames: u32,
	},
	/// Drops every voice of a sound being unloaded, at once.
	Cut {
		voices: [u32; 8],
		count: u8,
	},
}

/// A voice the mixer is done with, sent back to be freed.
#[derive(Debug)]
pub struct Retired {
	pub voice: VoiceState,
	/// Played to its end, rather than being stopped.
	pub ended: bool,
}

/// The control thread's ends of the mixer's queues.
pub struct Handle {
	pub commands: Producer<Command>,
	pub retired: Consumer<Retired>,
}

pub struct Mixer {
	rate: f32,
	commands: Consumer<Command>,
	retired: Producer<Retired>,
	voices: Box<[Option<VoiceState>]>,
	/// Voices finished while the retired queue was full.
	finished: [bool; MAX_VOICES],
	volume: Ramp,
	filters: [FilterSlot; SLOTS],
	hold: Ramp,
	held: bool,
	block: Box<[f32]>,
}

impl Mixer {
	/// A mixer at `rate`, and the control thread's handle to it.
	pub fn new(rate: u32) -> (Self, Handle) {
		let (commands, command_queue) = RingBuffer::new(COMMAND_CAPACITY);
		// Every voice the mixer can hold fits, so retiring never waits.
		let (retired_queue, retired) = RingBuffer::new(MAX_VOICES);
		let output = rate as f32;
		let mixer = Self {
			rate: output,
			commands: command_queue,
			retired: retired_queue,
			voices: (0..MAX_VOICES).map(|_| None).collect(),
			finished: [false; MAX_VOICES],
			volume: Ramp::new(1.0),
			filters: [FilterSlot::new(output); SLOTS],
			hold: Ramp::new(1.0),
			held: false,
			block: vec![0.0; BLOCK * 2].into_boxed_slice(),
		};

		(mixer, Handle { commands, retired })
	}

	/// Fills an interleaved buffer of `channels` channels.
	pub fn render(&mut self, output: &mut [f32], channels: usize) {
		self.apply_commands();
		let channels = channels.max(1);

		for chunk in output.chunks_mut(BLOCK * channels) {
			let frames = chunk.len() / channels;
			self.mix(frames);
			spread(&self.block[..frames * 2], chunk, channels);
		}
	}

	fn mix(&mut self, frames: usize) {
		self.block[..frames * 2].fill(0.0);

		if self.held && self.hold.settled() {
			return;
		}

		self.retire_finished();
		self.mix_voices(frames);
		self.master(frames);
	}

	fn mix_voices(&mut self, frames: usize) {
		let Self {
			voices,
			finished,
			retired,
			block,
			..
		} = self;
		let block = &mut block[..frames * 2];

		for (slot, finished) in voices.iter_mut().zip(finished.iter_mut()) {
			let Some(voice) = slot.as_mut() else { continue };

			if !*finished && !voice.render(block) {
				retire(slot, finished, retired);
			}
		}
	}

	/// The master bus: its filters, volume and hold, then a hard limit.
	fn master(&mut self, frames: usize) {
		for frame in self.block[..frames * 2].as_chunks_mut::<2>().0 {
			let mut sample = *frame;

			for filter in &mut self.filters {
				sample = filter.process(sample);
			}

			let gain = self.volume.next() * self.hold.next();
			*frame = sample.map(|sample| (sample * gain).clamp(-1.0, 1.0));
		}
	}

	/// Retries voices that finished while the retired queue was full.
	fn retire_finished(&mut self) {
		for (slot, finished) in self.voices.iter_mut().zip(self.finished.iter_mut()) {
			if *finished {
				retire(slot, finished, &mut self.retired);
			}
		}
	}

	fn apply_commands(&mut self) {
		while let Ok(command) = self.commands.pop() {
			self.apply(command);
		}
	}

	fn apply(&mut self, command: Command) {
		match command {
			Command::Play(voice) => self.play(voice),
			Command::Volume {
				target,
				volume,
				frames,
			} => self.set_volume(target, volume, frames),
			Command::Stop { target, frames } => self.each(target, |voice| voice.stop(frames)),
			Command::Pause {
				target,
				paused,
				frames,
			} => self.pause(target, paused, frames),
			Command::Filter {
				target,
				slot,
				filter,
				frames,
			} => self.set_filter(target, slot, filter, frames),
			Command::Cut { voices, count } => self.cut(&voices[..usize::from(count)]),
		}
	}

	/// Stops voices at once, with only the click-free minimum fade.
	fn cut(&mut self, voices: &[u32]) {
		let fade = frames(0.0, self.rate);

		for id in voices {
			self.each(*id, |voice| voice.stop(fade));
		}
	}

	fn play(&mut self, voice: VoiceState) {
		// The control thread never has more voices out than there are slots.
		if let Some(slot) = self.voices.iter_mut().find(|slot| slot.is_none()) {
			*slot = Some(voice);
		} else {
			let _ = self.retired.push(Retired {
				voice,
				ended: false,
			});
		}
	}

	/// Runs `change` on voice `target`, or on every voice for the master.
	fn each(&mut self, target: Target, change: impl FnMut(&mut VoiceState)) {
		self.voices
			.iter_mut()
			.flatten()
			.filter(|voice| target == 0 || voice.id == target)
			.for_each(change);
	}

	fn set_volume(&mut self, target: Target, volume: f32, frames: u32) {
		if target == 0 {
			self.volume.set(volume, frames);
		} else {
			self.each(target, |voice| voice.volume.set(volume, frames));
		}
	}

	fn set_filter(&mut self, target: Target, slot: usize, filter: Option<Filter>, frames: u32) {
		if target == 0 {
			self.filters[slot].set(filter, frames);
		} else {
			self.each(target, |voice| voice.filters[slot].set(filter, frames));
		}
	}

	fn pause(&mut self, target: Target, paused: bool, frames: u32) {
		let level = if paused { 0.0 } else { 1.0 };

		if target == 0 {
			self.held = paused;
			self.hold.set(level, frames);
		} else {
			self.each(target, |voice| {
				voice.paused = paused;
				voice.pause.set(level, frames);
			});
		}
	}
}

/// Sends a voice back to be freed. When the queue is full it stays, marked
/// finished, to be sent on a later callback.
fn retire(slot: &mut Option<VoiceState>, finished: &mut bool, retired: &mut Producer<Retired>) {
	let Some(voice) = slot.take() else { return };
	let ended = !voice.stopping;

	*finished = match retired.push(Retired { voice, ended }) {
		Ok(()) => false,
		Err(rtrb::PushError::Full(back)) => {
			*slot = Some(back.voice);
			true
		}
	};
}

/// Copies the stereo mix into an output of any channel count: mono takes
/// the average, and channels past the second stay silent.
fn spread(block: &[f32], output: &mut [f32], channels: usize) {
	for (frame, [left, right]) in output
		.chunks_exact_mut(channels)
		.zip(block.as_chunks::<2>().0)
	{
		if channels == 1 {
			frame[0] = f32::midpoint(*left, *right);
		} else {
			frame[0] = *left;
			frame[1] = *right;
			frame[2..].fill(0.0);
		}
	}
}

#[cfg(test)]
mod tests;
