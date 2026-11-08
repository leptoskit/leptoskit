//! The native engine behind the `audio` plugin.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use leptoskit_bridge::AppContext;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;

mod dsp;
mod mixer;
mod output;
mod platform;
mod source;

use mixer::Command;
use mixer::Feed;
use mixer::Handle;
use mixer::MAX_VOICES;
use mixer::Mixer;
use mixer::Start;
use mixer::VoiceState;
use output::Output;
use platform::Events;
use source::Decoder;
use source::Encoded;
use source::Pcm;
use source::STREAM_THRESHOLD;
use source::Streamer;

use crate::Event;
use crate::FILTER_SLOTS;
use crate::Filter;
use crate::LoadMode;
use crate::METHODS;
use crate::NAME;
use crate::Play;
use crate::Session;
use crate::Sound;
use crate::SoundInfo;
use crate::Source;
use crate::Target;
use crate::Voice;
use crate::protocol;
use crate::protocol::Request;

/// Constructs the plugin from the host [`AppContext`]. Nothing is opened
/// until the first sound plays.
pub fn from_context(context: &AppContext) -> Result<Audio, Error> {
	Ok(Audio::new(context.clone(), None))
}

/// A plugin whose mixer runs without a device, and the mixer: call
/// [`Rendering::render`] to pull its output. For tests, and for rendering
/// sound to a file.
pub fn offline(context: &AppContext, rate: u32) -> (Audio, Rendering) {
	let (mixer, handle) = Mixer::new(rate);
	let audio = Audio::new(
		context.clone(),
		Some(Engine {
			output: None,
			handle,
			rate,
		}),
	);
	(audio, Rendering(mixer))
}

/// The mixer of an [`offline`] plugin.
pub struct Rendering(Mixer);

impl std::fmt::Debug for Rendering {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Rendering").finish_non_exhaustive()
	}
}

impl Rendering {
	/// Fills an interleaved buffer of `channels` channels.
	pub fn render(&mut self, output: &mut [f32], channels: usize) {
		self.0.render(output, channels);
	}
}

/// A mixer and the device it plays on.
struct Engine {
	output: Option<Output>,
	handle: Handle,
	rate: u32,
}

enum Loaded {
	Memory(Arc<Pcm>),
	Stream(Encoded),
}

/// The master bus as the guest last set it, replayed onto a new output.
#[derive(Debug, Clone, Copy)]
struct Master {
	volume: f32,
	filters: [Option<Filter>; FILTER_SLOTS as usize],
	held: bool,
}

/// The `audio` plugin: sounds, voices and the engine that plays them.
pub struct Audio {
	context: AppContext,
	engine: Option<Engine>,
	offline: bool,
	streamer: Option<Streamer>,
	sounds: HashMap<u32, Loaded>,
	/// Voices in the mixer, and their sounds.
	voices: HashMap<u32, u32>,
	next_sound: u32,
	next_voice: u32,
	master: Master,
	events: Events,
	session: platform::State,
}

impl std::fmt::Debug for Audio {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Audio")
			.field("sounds", &self.sounds.len())
			.field("voices", &self.voices.len())
			.field("master", &self.master)
			.finish_non_exhaustive()
	}
}

impl Audio {
	fn new(context: AppContext, engine: Option<Engine>) -> Self {
		Self {
			context,
			offline: engine.is_some(),
			engine,
			streamer: None,
			sounds: HashMap::new(),
			voices: HashMap::new(),
			next_sound: 1,
			next_voice: 1,
			master: Master {
				volume: 1.0,
				filters: [None; FILTER_SLOTS as usize],
				held: false,
			},
			events: Events::default(),
			session: platform::State::default(),
		}
	}

	/// Handles one decoded request.
	pub fn handle(&mut self, request: Request<'_>) -> Result<Bytes, Error> {
		self.reap();

		match request {
			Request::Load(source, mode) => self.load(source, mode).map(protocol::encode_sound_info),
			Request::Play(sound, play) => self.play(sound, play).map(protocol::encode_voice),
			Request::TakeEvents => self.take_events(),
			request => self.change(request).map(|()| Bytes::new()),
		}
	}

	/// The requests that change something and answer nothing.
	fn change(&mut self, request: Request<'_>) -> Result<(), Error> {
		match request {
			Request::Configure(session) => self.configure(session),
			Request::Unload(sound) => self.unload(sound),
			Request::SetVolume(target, volume, ramp) => self.set_volume(target, volume, ramp),
			Request::Stop(target, fade) => {
				self.send_timed(target, fade, |target, frames| {
					Command::Stop { target, frames }
				})
			}
			Request::Pause(target, paused, fade) => self.pause(target, paused, fade),
			Request::SetFilter(target, slot, filter, glide) => {
				self.set_filter(target, slot as usize, filter, glide)
			}
			Request::Load(..) | Request::Play(..) | Request::TakeEvents => Err(Error::Unavailable),
		}
	}

	fn take_events(&self) -> Result<Bytes, Error> {
		let events: Vec<Event> = self
			.events
			.lock()
			.map(|mut events| events.drain(..).collect())
			.unwrap_or_default();
		protocol::encode_events(events.into_iter())
	}

	fn set_volume(&mut self, target: Target, volume: f32, ramp: Duration) -> Result<(), Error> {
		if target == Target::Master {
			self.master.volume = volume;
		}

		self.send_timed(target, ramp, |target, frames| {
			Command::Volume {
				target,
				volume,
				frames,
			}
		})
	}

	fn set_filter(
		&mut self,
		target: Target,
		slot: usize,
		filter: Option<Filter>,
		glide: Duration,
	) -> Result<(), Error> {
		if target == Target::Master {
			self.master.filters[slot] = filter;
		}

		self.send_timed(target, glide, |target, frames| {
			Command::Filter {
				target,
				slot,
				filter,
				frames,
			}
		})
	}

	fn configure(&mut self, session: Session) -> Result<(), Error> {
		platform::configure(&self.context, session, &self.events, &mut self.session)
	}

	/// Frees what the mixer is done with, reports ended voices, and drops an
	/// output whose device went away.
	fn reap(&mut self) {
		let Some(engine) = &mut self.engine else {
			return;
		};

		while let Ok(retired) = engine.handle.retired.pop() {
			self.voices.remove(&retired.voice.id);

			if retired.ended {
				platform::push(&self.events, Event::Ended(Voice(retired.voice.id)));
			}
		}

		if engine.output.as_ref().is_some_and(Output::lost) {
			// The voices went with the old stream; the next play reopens.
			self.engine = None;
			self.voices.clear();
		}
	}

	fn engine(&mut self) -> Result<&mut Engine, Error> {
		if self.engine.is_none() {
			if self.offline {
				return Err(Error::Unavailable);
			}

			platform::prepare(&self.context)?;
			let (output, handle) = Output::open()?;
			let rate = output.rate;
			let mut engine = Engine {
				output: Some(output),
				handle,
				rate,
			};
			restore(&mut engine, self.master)?;
			self.engine = Some(engine);
		}

		self.engine.as_mut().ok_or(Error::Unavailable)
	}

	fn load(&mut self, source: Source<'_>, mode: LoadMode) -> Result<SoundInfo, Error> {
		let encoded = match source {
			Source::Asset(path) => platform::asset(&self.context, path)?,
			Source::Data(path) => Encoded::File(self.context.data_dir.join(path)),
			Source::Cache(path) => Encoded::File(self.context.cache_dir.join(path)),
			Source::Bytes(bytes) => Encoded::Bytes(bytes.into()),
		};

		let streamed = match mode {
			LoadMode::Auto => encoded.len()? > STREAM_THRESHOLD,
			LoadMode::Memory => false,
			LoadMode::Stream => true,
		};

		let decoder = Decoder::open(&encoded)?;
		let mut shape = decoder.shape();
		let loaded = if streamed {
			Loaded::Stream(encoded)
		} else {
			let pcm = decoder.decode_all()?;

			shape.frames = Some(pcm.frames() as u64);
			Loaded::Memory(Arc::new(pcm))
		};

		let sound = self.next_sound;
		self.next_sound = self.next_sound.checked_add(1).ok_or(Error::Exhausted)?;
		self.sounds.insert(sound, loaded);

		Ok(SoundInfo {
			sound: Sound(sound),
			sample_rate: shape.rate,
			channels: shape.channels,
			frames: shape.frames,
			streamed,
		})
	}

	fn unload(&mut self, sound: Sound) -> Result<(), Error> {
		self.sounds.remove(&sound.0).ok_or(Error::InvalidArgument)?;
		let voices: Vec<u32> = self
			.voices
			.iter()
			.filter(|&(_, &owner)| owner == sound.0)
			.map(|(&voice, _)| voice)
			.collect();

		for batch in voices.chunks(8) {
			let mut ids = [0; 8];
			ids[..batch.len()].copy_from_slice(batch);
			self.send(Command::Cut {
				voices: ids,
				count: batch.len() as u8,
			})?;
		}

		Ok(())
	}

	fn play(&mut self, sound: Sound, play: Play) -> Result<Voice, Error> {
		if self.voices.len() >= MAX_VOICES {
			return Err(Error::Busy);
		}

		let (feed, rate) = self.feed(sound, play.looping)?;
		let voice = self.next_voice;
		let engine = self.engine()?;
		let state = VoiceState::new(
			voice,
			feed,
			Start {
				source_rate: rate,
				volume: play.volume,
				pan: play.pan,
				looping: play.looping,
				fade_in: play.fade_in.as_secs_f32(),
				paused: play.paused,
			},
			engine.rate,
		);
		engine
			.handle
			.commands
			.push(Command::Play(state))
			.map_err(|_| Error::Busy)?;
		self.next_voice = voice.checked_add(1).ok_or(Error::Exhausted)?;
		self.voices.insert(voice, sound.0);

		Ok(Voice(voice))
	}

	/// Where a new voice of `sound` reads from, and at what rate: a share of
	/// the decoded sound, or a stream opened for it alone.
	fn feed(&mut self, sound: Sound, looping: bool) -> Result<(Feed, u32), Error> {
		match self.sounds.get(&sound.0).ok_or(Error::InvalidArgument)? {
			Loaded::Memory(pcm) => {
				let feed = Feed::Memory {
					pcm: Arc::clone(pcm),
					frame: 0,
				};
				Ok((feed, pcm.rate()))
			}
			Loaded::Stream(encoded) => {
				let encoded = encoded.clone();
				let streamer = match &mut self.streamer {
					Some(streamer) => streamer,
					streamer => streamer.insert(Streamer::spawn()?),
				};

				let (feed, shape) = streamer.feed(&encoded, looping)?;
				Ok((feed, shape.rate))
			}
		}
	}

	/// Holding the master also lets the device sleep once the fade is done.
	fn pause(&mut self, target: Target, paused: bool, fade: Duration) -> Result<(), Error> {
		if target == Target::Master {
			self.master.held = paused;
			self.hold_output(paused, fade);
		}

		self.send_timed(target, fade, |target, frames| {
			Command::Pause {
				target,
				paused,
				frames,
			}
		})
	}

	fn hold_output(&self, held: bool, fade: Duration) {
		let Some(output) = self
			.engine
			.as_ref()
			.and_then(|engine| engine.output.as_ref())
		else {
			return;
		};

		if held {
			output.hold(fade);
		} else {
			output.resume();
		}
	}

	/// Sends a timed command, once the target is known to exist. Voices that
	/// already ended accept it and nothing happens.
	fn send_timed(
		&mut self,
		target: Target,
		time: Duration,
		command: impl FnOnce(u32, u32) -> Command,
	) -> Result<(), Error> {
		let target = match target {
			Target::Master => 0,

			Target::Voice(Voice(id)) if self.voices.contains_key(&id) => id,

			Target::Voice(Voice(id)) if id < self.next_voice => return Ok(()),
			Target::Voice(_) => return Err(Error::InvalidArgument),
		};

		// The master bus changes before anything plays are kept for later.
		let Some(engine) = &mut self.engine else {
			return Ok(());
		};
		let frames = dsp::frames(time.as_secs_f32(), engine.rate as f32);
		self.send(command(target, frames))
	}

	fn send(&mut self, command: Command) -> Result<(), Error> {
		match &mut self.engine {
			Some(engine) => {
				engine
					.handle
					.commands
					.push(command)
					.map_err(|_| Error::Busy)
			}
			None => Ok(()),
		}
	}
}

/// Replays the master bus onto a new output.
fn restore(engine: &mut Engine, master: Master) -> Result<(), Error> {
	let commands = &mut engine.handle.commands;
	let mut send = |command| commands.push(command).map_err(|_| Error::Busy);
	send(Command::Volume {
		target: 0,
		volume: master.volume,
		frames: 0,
	})?;

	for (slot, filter) in master.filters.into_iter().enumerate() {
		if filter.is_some() {
			send(Command::Filter {
				target: 0,
				slot,
				filter,
				frames: 0,
			})?;
		}
	}

	if master.held {
		send(Command::Pause {
			target: 0,
			paused: true,
			frames: 0,
		})?;
	}

	Ok(())
}

impl Plugin for Audio {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		let request = protocol::decode(method, &payload)?;
		self.handle(request)
	}
}

#[cfg(test)]
mod tests;
