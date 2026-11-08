//! One voice's nodes: its player, two filter slots, volume, fade and pan.

use leptoskit_bridge::Error;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::AudioBuffer;
use web_sys::AudioBufferSourceNode;
use web_sys::AudioContext;
use web_sys::AudioNode;
use web_sys::BiquadFilterNode;
use web_sys::GainNode;
use web_sys::HtmlAudioElement;
use web_sys::StereoPannerNode;

use super::graph::MIN_RAMP;
use super::graph::halt;
use super::graph::later;
use super::graph::native;
use super::graph::ramp;
use super::graph::start_buffer;
use super::graph::transparent;
use crate::FILTER_SLOTS;
use crate::Play;

/// What a sound is in the browser.
pub enum Kind {
	/// Decoded, for effects.
	Buffer(AudioBuffer),
	/// Streamed by an `<audio>` element from this address.
	Stream(String),
}

#[allow(variant_size_differences)]
enum Player {
	Buffer {
		node: AudioBufferSourceNode,
		buffer: AudioBuffer,
		/// When the current node started, in context time, and from where.
		started: f64,
		offset: f64,
	},
	Element(HtmlAudioElement),
}

pub struct WebVoice {
	/// The sound it plays, so unloading the sound can stop it.
	pub sound: u32,
	player: Player,
	looping: bool,
	pub paused: bool,
	pub stopping: bool,
	pub filters: [BiquadFilterNode; FILTER_SLOTS as usize],
	pub volume: GainNode,
	fade: GainNode,
	panner: StereoPannerNode,
	ended: Closure<dyn FnMut()>,
}

impl WebVoice {
	/// Builds the voice's chain into `output` and starts it, unless it
	/// starts paused. `ended` runs when its player finishes.
	pub fn start(
		context: &AudioContext,
		kind: &Kind,
		play: Play,
		output: &AudioNode,
		ended: Closure<dyn FnMut()>,
	) -> Result<Self, Error> {
		let filters = [transparent(context)?, transparent(context)?];
		let volume = context.create_gain().map_err(native)?;
		let fade = context.create_gain().map_err(native)?;
		let panner = context.create_stereo_panner().map_err(native)?;
		let chain: [&AudioNode; 5] = [&filters[0], &filters[1], &volume, &fade, &panner];

		for pair in chain.windows(2) {
			pair[0].connect_with_audio_node(pair[1]).map_err(native)?;
		}

		panner.connect_with_audio_node(output).map_err(native)?;
		volume.gain().set_value(play.volume);
		panner.pan().set_value(play.pan);
		let now = context.current_time();
		let silent = !play.fade_in.is_zero();

		fade.gain().set_value(if silent { 0.0 } else { 1.0 });
		ramp(&fade.gain(), 1.0, now, play.fade_in.as_secs_f64());
		let player = player(context, kind, play.looping, &filters[0], &ended)?;
		let mut voice = Self {
			sound: 0,
			player,
			looping: play.looping,
			paused: false,
			stopping: false,
			filters,
			volume,
			fade,
			panner,
			ended,
		};

		if play.paused {
			voice.pause(context, 0.0);
		} else if let Player::Element(element) = &voice.player {
			let _ = element.play();
		}

		Ok(voice)
	}

	/// Fades out over `time` and remembers where it stopped.
	pub fn pause(&mut self, context: &AudioContext, time: f64) {
		let now = context.current_time();
		self.paused = true;
		ramp(&self.fade.gain(), 0.0, now, time);
		let end = now + time.max(MIN_RAMP);

		match &mut self.player {
			Player::Buffer {
				node,
				buffer,
				started,
				offset,
			} => {
				halt(node, end);
				*offset = position(*offset + (end - *started), buffer.duration(), self.looping);
			}
			Player::Element(element) => {
				let element = element.clone();
				later(time.max(MIN_RAMP), move || {
					let _ = element.pause();
				});
			}
		}
	}

	/// Carries on from where it paused, fading in over `time`.
	pub fn resume(&mut self, context: &AudioContext, time: f64) -> Result<(), Error> {
		let now = context.current_time();
		self.paused = false;
		ramp(&self.fade.gain(), 1.0, now, time);

		match &mut self.player {
			Player::Buffer {
				node,
				buffer,
				started,
				offset,
			} => {
				*node = start_buffer(
					context,
					buffer,
					&self.filters[0],
					self.looping,
					*offset,
					&self.ended,
				)?;
				*started = now;
			}
			Player::Element(element) => {
				let _ = element.play();
			}
		}

		Ok(())
	}

	/// Fades out over `time`. A buffer stops itself and reports through
	/// `ended`; an element is removed with `remove` after the fade.
	pub fn stop(&mut self, context: &AudioContext, time: f64, remove: impl FnOnce() + 'static) {
		let now = context.current_time();
		self.stopping = true;
		ramp(&self.fade.gain(), 0.0, now, time);

		match &self.player {
			Player::Buffer { node, .. } if !self.paused => {
				let _ = super::graph::scheduled(node).stop_with_when(now + time.max(MIN_RAMP));
			}

			_ => later(time.max(MIN_RAMP), remove),
		}
	}

	/// Silences and detaches everything.
	pub fn release(&self) {
		if let Player::Element(element) = &self.player {
			let _ = element.pause();
		}

		let _ = self.panner.disconnect();
	}
}

/// Where playback stands after `played` seconds of a sound `length` long.
fn position(played: f64, length: f64, looping: bool) -> f64 {
	if looping && length > 0.0 {
		played % length
	} else {
		played.min(length)
	}
}

fn player(
	context: &AudioContext,
	kind: &Kind,
	looping: bool,
	into: &BiquadFilterNode,
	ended: &Closure<dyn FnMut()>,
) -> Result<Player, Error> {
	match kind {
		Kind::Buffer(buffer) => {
			Ok(Player::Buffer {
				node: start_buffer(context, buffer, into, looping, 0.0, ended)?,
				buffer: buffer.clone(),
				started: context.current_time(),
				offset: 0.0,
			})
		}
		Kind::Stream(url) => {
			let element = HtmlAudioElement::new_with_src(url).map_err(native)?;
			element.set_loop(looping);
			element.set_onended(Some(ended.as_ref().unchecked_ref()));
			context
				.create_media_element_source(&element)
				.map_err(native)?
				.connect_with_audio_node(into)
				.map_err(native)?;
			Ok(Player::Element(element))
		}
	}
}
