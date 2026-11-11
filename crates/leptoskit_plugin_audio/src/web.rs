//! Web Audio for browser apps, with the same model as the native engine:
//! sounds, voices, ramps, filters and a master bus.
//!
//! Short sounds are decoded into `AudioBuffer`s and played by
//! `AudioBufferSourceNode`s; streamed sounds play from an `<audio>` element
//! routed into the same graph. Each voice runs through two filter slots, a
//! volume gain, a fade gain and a stereo panner into the master bus's own
//! filters and gain. Browsers start an `AudioContext` suspended until the
//! page has had a user gesture, so the context resumes itself on the first
//! pointer or key press.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::rc::Rc;
use std::rc::Weak;
use std::time::Duration;

use js_sys::ArrayBuffer;
use leptoskit_bridge::Error;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use web_sys::AudioBuffer;
use web_sys::AudioContext;
use web_sys::AudioContextState;
use web_sys::AudioNode;
use web_sys::BiquadFilterNode;
use web_sys::Document;
use web_sys::GainNode;

use self::graph::MIN_RAMP;
use self::graph::fetch;
use self::graph::later;
use self::graph::native;
use self::graph::ramp;
use self::graph::seconds;
use self::graph::transparent;
use self::voice::Kind;
use self::voice::WebVoice;
use super::*;

mod graph;
#[cfg(test)]
mod tests;
mod voice;

/// Events a gesture can unlock audio with.
const UNLOCK_EVENTS: [&str; 3] = ["pointerdown", "keydown", "touchend"];
/// Assets larger than this stream in [`LoadMode::Auto`].
const STREAM_THRESHOLD: u64 = 512 * 1024;
/// The most events kept; the oldest go first.
const MAX_EVENTS: usize = 1_024;

struct Inner {
	context: AudioContext,
	input: AudioNode,
	filters: [BiquadFilterNode; FILTER_SLOTS as usize],
	volume: GainNode,
	sounds: RefCell<HashMap<u32, (Kind, SoundInfo)>>,
	voices: RefCell<HashMap<u32, WebVoice>>,
	events: RefCell<VecDeque<Event>>,
	next_sound: Cell<u32>,
	next_voice: Cell<u32>,
	unlock: RefCell<Option<Closure<dyn FnMut()>>>,
}

/// The browser engine. Clones share one `AudioContext`.
#[derive(Clone)]
pub struct Audio(Rc<Inner>);

impl std::fmt::Debug for Audio {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Audio")
			.field("voices", &self.0.voices.borrow().len())
			.finish_non_exhaustive()
	}
}

impl Audio {
	/// Creates the engine. Audio starts on the page's first user gesture if
	/// the browser held it back.
	pub fn new() -> Result<Self, Error> {
		let context = AudioContext::new().map_err(|_| Error::Unavailable)?;
		Self::with_context(context)
	}

	fn with_context(context: AudioContext) -> Result<Self, Error> {
		let filters = [transparent(&context)?, transparent(&context)?];
		let volume = context.create_gain().map_err(native)?;
		filters[0]
			.connect_with_audio_node(&filters[1])
			.map_err(native)?;
		filters[1]
			.connect_with_audio_node(&volume)
			.map_err(native)?;
		volume
			.connect_with_audio_node(&context.destination())
			.map_err(native)?;
		let audio = Self(Rc::new(Inner {
			input: filters[0].clone().into(),
			context,
			filters,
			volume,
			sounds: RefCell::default(),
			voices: RefCell::default(),
			events: RefCell::default(),
			next_sound: Cell::new(1),
			next_voice: Cell::new(1),
			unlock: RefCell::new(None),
		}));
		audio.listen_for_unlock();

		Ok(audio)
	}

	/// Whether the browser lets the context play yet.
	pub fn unlocked(&self) -> bool {
		self.0.context.state() == AudioContextState::Running
	}

	/// Resumes the context on the first gesture, then stops listening.
	fn listen_for_unlock(&self) {
		let Some(document) = page().filter(|_| !self.unlocked()) else {
			return;
		};
		let weak = Rc::downgrade(&self.0);
		let closure = Closure::<dyn FnMut()>::new(move || {
			let Some(inner) = weak.upgrade() else { return };
			let _ = inner.context.resume();

			if let (Some(document), Some(closure)) = (page(), inner.unlock.borrow().as_ref()) {
				listen(&document, closure, false);
			}
		});
		listen(&document, &closure, true);
		*self.0.unlock.borrow_mut() = Some(closure);
	}

	/// The browser has no audio session; this does nothing.
	pub fn configure(&self, session: Session) -> Result<(), Error> {
		let _ = session;
		Ok(())
	}

	/// Loads a sound. Assets come from `/assets/<path>`; in
	/// [`LoadMode::Auto`] those larger than 512 KiB stream through an
	/// `<audio>` element. The app's data and cache folders do not exist in
	/// the browser.
	pub async fn load(&self, source: Source<'_>, mode: LoadMode) -> Result<SoundInfo, Error> {
		let (kind, frames) = match source {
			Source::Asset(path) => self.load_asset(path, mode).await?,
			Source::Bytes(bytes) => {
				self.decode(js_sys::Uint8Array::from(bytes).buffer())
					.await?
			}
			Source::Data(_) | Source::Cache(_) => return Err(Error::Unavailable),
		};

		let sound = self.0.next_sound.get();
		self.0.next_sound.set(sound + 1);

		let (sample_rate, channels) = match &kind {
			Kind::Buffer(buffer) => (buffer.sample_rate() as u32, buffer.number_of_channels()),
			Kind::Stream(_) => (self.0.context.sample_rate() as u32, 2),
		};

		let info = SoundInfo {
			sound: Sound(sound),
			sample_rate,
			channels,
			frames,
			streamed: matches!(kind, Kind::Stream(_)),
		};
		self.0.sounds.borrow_mut().insert(sound, (kind, info));
		Ok(info)
	}

	async fn load_asset(&self, path: &str, mode: LoadMode) -> Result<(Kind, Option<u64>), Error> {
		let url = format!("/assets/{}", check_path(path)?);
		let response = fetch(&url).await?;
		let length = response
			.headers()
			.get("content-length")
			.ok()
			.flatten()
			.and_then(|length| length.parse::<u64>().ok());

		let streamed = match mode {
			LoadMode::Auto => length.is_some_and(|length| length > STREAM_THRESHOLD),
			LoadMode::Memory => false,
			LoadMode::Stream => true,
		};

		if streamed {
			return Ok((Kind::Stream(url), None));
		}

		let bytes = JsFuture::from(response.array_buffer().map_err(native)?)
			.await
			.map_err(native)?;
		self.decode(bytes.unchecked_into()).await
	}

	async fn decode(&self, bytes: ArrayBuffer) -> Result<(Kind, Option<u64>), Error> {
		let buffer: AudioBuffer =
			JsFuture::from(self.0.context.decode_audio_data(&bytes).map_err(native)?)
				.await
				.map_err(|_| Error::InvalidArgument)?
				.unchecked_into();
		let frames = u64::from(buffer.length());
		Ok((Kind::Buffer(buffer), Some(frames)))
	}

	/// Releases a sound and stops its voices at once.
	pub fn unload(&self, sound: Sound) -> Result<(), Error> {
		self.0
			.sounds
			.borrow_mut()
			.remove(&sound.0)
			.ok_or(Error::InvalidArgument)?;
		let voices: Vec<u32> = self
			.0
			.voices
			.borrow()
			.iter()
			.filter(|(_, voice)| voice.sound == sound.0)
			.map(|(&id, _)| id)
			.collect();
		voices
			.into_iter()
			.try_for_each(|id| self.stop(Target::Voice(Voice(id)), Duration::ZERO))
	}

	/// Starts a voice.
	pub fn play(&self, sound: Sound, play: Play) -> Result<Voice, Error> {
		check_volume(play.volume)?;
		check_pan(play.pan)?;
		let id = self.0.next_voice.get();
		let weak = Rc::downgrade(&self.0);
		let ended = Closure::<dyn FnMut()>::new(move || finished(&weak, id));
		let mut voice = {
			let sounds = self.0.sounds.borrow();
			let (kind, _) = sounds.get(&sound.0).ok_or(Error::InvalidArgument)?;
			WebVoice::start(&self.0.context, kind, play, &self.0.input, ended)?
		};
		voice.sound = sound.0;
		self.0.next_voice.set(id + 1);

		self.0.voices.borrow_mut().insert(id, voice);
		Ok(Voice(id))
	}

	/// Runs `change` on a playing voice. A voice that already ended is
	/// accepted and left alone; one never started is refused.
	fn with_voice(
		&self,
		voice: Voice,
		change: impl FnOnce(&mut WebVoice) -> Result<(), Error>,
	) -> Result<(), Error> {
		if let Some(playing) = self.0.voices.borrow_mut().get_mut(&voice.0) {
			return change(playing);
		}

		if voice.0 != 0 && voice.0 < self.0.next_voice.get() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}

	/// Ramps a voice's volume, or the master volume.
	pub fn set_volume(&self, target: Target, volume: f32, time: Duration) -> Result<(), Error> {
		check_volume(volume)?;
		let now = self.0.context.current_time();

		let Target::Voice(voice) = target else {
			ramp(&self.0.volume.gain(), volume, now, seconds(time));

			return Ok(());
		};

		self.with_voice(voice, |voice| {
			ramp(&voice.volume.gain(), volume, now, seconds(time));
			Ok(())
		})
	}

	/// Fades out and stops a voice, or every voice.
	pub fn stop(&self, target: Target, fade: Duration) -> Result<(), Error> {
		let Target::Voice(voice) = target else {
			let ids: Vec<u32> = self.0.voices.borrow().keys().copied().collect();

			return ids
				.into_iter()
				.try_for_each(|id| self.stop(Target::Voice(Voice(id)), fade));
		};
		let weak = Rc::downgrade(&self.0);

		self.with_voice(voice, |playing| {
			playing.stop(&self.0.context, seconds(fade), move || {
				remove(&weak, voice.0);
			});
			Ok(())
		})
	}

	/// Pauses or resumes a voice, or the whole context.
	pub fn pause(&self, target: Target, paused: bool, fade: Duration) -> Result<(), Error> {
		let time = seconds(fade);

		let Target::Voice(voice) = target else {
			self.hold(paused, time);

			return Ok(());
		};

		self.with_voice(voice, |playing| {
			match (paused, playing.paused) {
				(true, false) => playing.pause(&self.0.context, time),
				(false, true) => return playing.resume(&self.0.context, time),
				_ => {}
			}

			Ok(())
		})
	}

	/// Holds the whole output: the master fades out, then the context
	/// suspends so the device can sleep.
	fn hold(&self, held: bool, time: f64) {
		let context = &self.0.context;
		let now = context.current_time();

		if !held {
			let _ = context.resume();
			ramp(&self.0.volume.gain(), 1.0, now, time);

			return;
		}

		ramp(&self.0.volume.gain(), 0.0, now, time);
		let weak = Rc::downgrade(&self.0);
		later(time.max(MIN_RAMP), move || {
			if let Some(inner) = weak.upgrade() {
				let _ = inner.context.suspend();
			}
		});
	}

	/// Sets a filter slot of a voice or the master bus.
	pub fn set_filter(
		&self,
		target: Target,
		slot: u32,
		filter: Option<Filter>,
		glide: Duration,
	) -> Result<(), Error> {
		if slot >= FILTER_SLOTS {
			return Err(Error::InvalidArgument);
		}

		let filter = filter.map(check_filter).transpose()?;
		let rate = self.0.context.sample_rate();
		let now = self.0.context.current_time();
		let set =
			|node: &BiquadFilterNode| graph::set_filter(node, filter, rate, now, seconds(glide));

		let Target::Voice(voice) = target else {
			set(&self.0.filters[slot as usize]);

			return Ok(());
		};

		self.with_voice(voice, |voice| {
			set(&voice.filters[slot as usize]);
			Ok(())
		})
	}

	/// Starts `sound` rising over `fade` while `from` falls away.
	pub fn crossfade(
		&self,
		from: Option<Voice>,
		sound: Sound,
		play: Play,
		fade: Duration,
	) -> Result<Voice, Error> {
		let voice = self.play(sound, play.fade_in(fade))?;

		if let Some(from) = from {
			self.stop(Target::Voice(from), fade)?;
		}

		Ok(voice)
	}

	/// Takes the events queued since the last call, oldest first.
	pub fn events(&self) -> Vec<Event> {
		self.0.events.borrow_mut().drain(..).collect()
	}
}

fn page() -> Option<Document> {
	web_sys::window()?.document()
}

/// Adds or removes the unlock listeners.
fn listen(document: &Document, closure: &Closure<dyn FnMut()>, add: bool) {
	let callback = closure.as_ref().unchecked_ref();

	for event in UNLOCK_EVENTS {
		let _ = if add {
			document.add_event_listener_with_callback(event, callback)
		} else {
			document.remove_event_listener_with_callback(event, callback)
		};
	}
}

/// A voice's player finished: report it unless it was stopped, and free it.
fn finished(inner: &Weak<Inner>, id: u32) {
	let Some(strong) = inner.upgrade() else {
		return;
	};
	let stopped = strong
		.voices
		.borrow()
		.get(&id)
		.is_none_or(|voice| voice.stopping);

	if !stopped {
		let mut events = strong.events.borrow_mut();

		if events.len() == MAX_EVENTS {
			events.pop_front();
		}

		events.push_back(Event::Ended(Voice(id)));
	}

	remove(inner, id);
}

fn remove(inner: &Weak<Inner>, id: u32) {
	let Some(inner) = inner.upgrade() else { return };
	let voice = inner.voices.borrow_mut().remove(&id);

	if let Some(voice) = voice {
		voice.release();
	}
}
