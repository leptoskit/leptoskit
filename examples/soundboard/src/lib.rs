//! A soundboard: one screen that loads an effect, plays it on a press, and
//! counts the voices that played to their end.
#![cfg(target_arch = "wasm32")]
#![allow(unsafe_code)] // Exported C names form the interpreter's entry points.

use leptoskit::components::Button;
use leptoskit::components::Text;
use leptoskit::components::View;
use leptoskit::guest::Error;
use leptoskit::view;
use leptoskit_plugin_audio::Event;
use leptoskit_plugin_audio::LoadMode;
use leptoskit_plugin_audio::Play;
use leptoskit_plugin_audio::Sound;
use leptoskit_plugin_audio::Source;
use leptoskit_plugin_audio::Voice;
use leptoskit_plugin_audio::guest::Audio;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
use reactive_graph::signal::RwSignal;

/// Half a second of a 440 Hz tone, embedded so the example needs no assets.
const STRIKE: &[u8] =
	include_bytes!("../../../crates/leptoskit_plugin_audio/tests/fixtures/tone.wav");

/// The effect's volume: a little under the recording's own level.
const STRIKE_VOLUME: f32 = 0.8;

/// Loads the effect and mounts the screen.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	let owner = Owner::new();
	let (sound, status, endings) = owner.with(|| {
		(
			RwSignal::new(None::<Sound>),
			RwSignal::new("Loading"),
			RwSignal::new(0_usize),
		)
	});

	leptoskit::guest::spawn(async move {
		match load().await {
			Ok(effect) => {
				sound.set(Some(effect));
				status.set("Ready");
			}
			Err(_) => status.set("No sound"),
		}
	});

	let on_strike = move || {
		let Some(effect) = sound.get_untracked() else {
			return;
		};
		leptoskit::guest::spawn(async move {
			let played = strike(effect).await;
			status.set(if played.is_ok() {
				"Playing"
			} else {
				"No sound"
			});
		});
	};

	let on_count = move || {
		leptoskit::guest::spawn(async move {
			if let Ok(ended) = ended().await {
				endings.update(|endings| *endings += ended);
			}
		});
	};

	leptoskit::guest::mount(move || {
		let _scope = &owner;
		view! {
			<View>
				<Text>{status.get()}</Text>
				<Button on:press=on_strike>"Strike"</Button>
				<Button on:press=on_count>"Count endings"</Button>
				<Text>"Endings: " {endings.get()}</Text>
			</View>
		}
	});
}

async fn load() -> Result<Sound, Error> {
	let audio = Audio::new()?;
	let info = audio.load(Source::Bytes(STRIKE), LoadMode::Memory).await?;
	Ok(info.sound)
}

async fn strike(effect: Sound) -> Result<Voice, Error> {
	Audio::new()?
		.play(effect, Play::new().volume(STRIKE_VOLUME))
		.await
}

/// How many voices played to their end since the last time.
async fn ended() -> Result<usize, Error> {
	let events = Audio::new()?.events().await?;
	Ok(events
		.iter()
		.filter(|event| matches!(event, Event::Ended(_)))
		.count())
}

/// Runs the tasks a plugin reply woke.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_poll() {
	leptoskit::guest::poll();
}

/// Forwards platform-neutral events to the shared tree's handlers.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	leptoskit::guest::dispatch(node, kind, ptr, len)
}

/// Allocates the interpreter's event payload buffer.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	leptoskit::guest::alloc(len)
}

/// Reports the native UI bridge ABI this guest implements.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit::guest::ABI_VERSION as i32
}
