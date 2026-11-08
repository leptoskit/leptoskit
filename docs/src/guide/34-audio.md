# Playing sound

A game needs music that loops without a gap, that fades from one piece to the next when the player changes rooms, and that goes muffled while a menu is open. It needs short effects that start the instant a button is pressed, several at once. And all of it has to stop when the app goes to the background, and come back when it returns. The `audio` plugin does that with one API on iOS, Android, macOS, and in the browser.

## Add the plugin

Add `leptoskit_plugin_audio` to your app with its `guest` feature, and to each native host, as with any [runtime plugin](../platforms/plugins.md). Then list it:

```toml
[native]
plugins = ["audio"]
```

## Music and an effect

This screen starts a looping piece of music when it opens, plays a short effect each time the button is pressed, and veils the music while a pause menu is showing:

```rust
use std::time::Duration;

use leptos::prelude::*;
use leptoskit::prelude::*;
use leptoskit_plugin_audio::Filter;
use leptoskit_plugin_audio::LoadMode;
use leptoskit_plugin_audio::Play;
use leptoskit_plugin_audio::Source;
use leptoskit_plugin_audio::Target;
use leptoskit_plugin_audio::guest::Audio;

#[component]
pub fn Hunt() -> impl IntoView {
	let paused = RwSignal::new(false);
	let swing = StoredValue::new(None);

	leptos::task::spawn_local(async move {
		let Ok(audio) = Audio::new() else { return };
		let Ok(music) = audio.load(Source::Asset("music/hunt.ogg"), LoadMode::Stream).await else {
			return;
		};
		let _ = audio
			.play(music.sound, Play::new().looping(true).fade_in(Duration::from_secs(2)))
			.await;

		if let Ok(effect) = audio.load(Source::Asset("effects/swing.wav"), LoadMode::Memory).await {
			swing.set_value(Some(effect.sound));
		}
	});

	let strike = move || {
		let Some(sound) = swing.get_value() else { return };
		leptos::task::spawn_local(async move {
			if let Ok(audio) = Audio::new() {
				let _ = audio.play(sound, Play::new().volume(0.8)).await;
			}
		});
	};

	let toggle_menu = move || {
		paused.update(|paused| *paused = !*paused);
		let veil = paused.get_untracked().then(|| Filter::low_pass(500.0));
		leptos::task::spawn_local(async move {
			if let Ok(audio) = Audio::new() {
				let glide = Duration::from_millis(400);
				let _ = audio.set_filter(Target::Master, 0, veil, glide).await;
				let quiet = if veil.is_some() { 0.5 } else { 1.0 };
				let _ = audio.set_volume(Target::Master, quiet, glide).await;
			}
		});
	};

	view! {
		<View class="flex-col gap-4 p-4">
			<Button on_press=strike>"Strike"</Button>
			<Button on_press=toggle_menu>{move || if paused.get() { "Resume" } else { "Pause" }}</Button>
		</View>
	}
}
```

What each call does:

- **`load`** reads a sound once and returns a handle. `LoadMode::Stream` is for music: it is decoded a second ahead while it plays, so a ten-minute piece never sits in memory decoded. `LoadMode::Memory` is for effects: decoded once, then every play starts at once and costs almost nothing. `LoadMode::Auto` streams files larger than 512 KiB. Ogg Vorbis, WAV, MP3, FLAC, and AAC in MP4 all load.
- **`play`** starts a voice and returns it. The same sound can play as many voices as you like, up to 64 at a time. A voice can loop, start at a volume and a stereo position, fade in, or start paused.
- **`set_filter`** puts a low-pass or high-pass filter in one of two slots, on one voice or on the master bus (`Target::Master`, which every voice passes through). The cutoff glides to its new value over the time you give, so the veil comes down smoothly; `None` glides the filter open and takes it out. `resonance` adds a peak at the cutoff.
- **`set_volume`** ramps a voice, or the master bus, to a new volume.

Every change takes at least five milliseconds, even one you ask for at once, so nothing clicks.

## Changing pieces and going quiet

`crossfade` brings a new piece in while the old one falls away over the same time:

```rust
let next = audio.load(Source::Asset("music/rift.ogg"), LoadMode::Stream).await?;
let rift = audio
	.crossfade(Some(hunt), next.sound, Play::new().looping(true), Duration::from_secs(3))
	.await?;
```

`stop(Target::Voice(voice), fade)` fades one voice out and ends it; `stop(Target::Master, fade)` silences everything. `pause(target, true, fade)` eases a voice, or the whole output, to silence and holds its place, and `pause(target, false, fade)` carries on from there. Pause the master bus when the app goes to the background and resume it when the app comes back: the device's output sleeps while it is held.

## Knowing when a sound ends

`events()` returns what happened since you last asked: `Event::Ended(voice)` when a voice that does not loop played to its end (a voice you stopped does not report it), and `Event::Interrupted` and `Event::Resumed` when the operating system took the audio away for a call or an alarm and gave it back. Ask once a frame, or whenever you need to know.

## Sharing the device with other apps

Ask for an audio session once, before playing:

```rust
audio
	.configure(Session {
		category: Category::Ambient,
		..Session::default()
	})
	.await?;
```

`Category::Ambient`, the default, is for game music and effects: it plays alongside other apps' audio and, on iPhone, goes quiet when the Ring/Silent switch is set to silent. `Category::Playback` is for sound the user asked to hear: it plays with the switch on silent and, unless you set `mix_with_others`, pauses other apps' audio; `duck_others` lowers them instead.

## On each platform

- **iOS:** sound plays through Core Audio. `configure` sets the `AVAudioSession` category, Ambient or Playback, with the options you chose. Interruptions arrive as events, and output restarts by itself when they end. Sounds stop when the app goes to the background unless you add the `audio` background mode to `Info.plist` and use `Category::Playback`.
- **Android:** sound plays through AAudio, at 48 kHz stereo. `Category::Playback` requests audio focus, so the system lowers your sound under navigation prompts and similar short sounds; `Category::Ambient`, and Playback with `mix_with_others`, request none. Losing focus to another app is not reported yet, so pause the master bus when your activity pauses.
- **macOS:** sound plays through Core Audio on the default output device. If that device goes away, the voices playing on it end and the next `play` opens the new default. There is no audio session.
- **Browser:** sound plays through Web Audio, with `web::Audio` in place of `guest::Audio`: the same calls, with `load` the only one you `await`. Assets come from `/assets/<path>`, which the web build serves from your `assets/` folder. Browsers keep audio silent until the page has been tapped or a key pressed; the plugin starts it on that first gesture. `Source::Data` and `Source::Cache` are unavailable. Streamed sounds play from an `<audio>` element.
- **Linux:** sound plays through ALSA on the default output device, and the generated app registers the plugin for you. Building the app needs ALSA's development files (`libasound2-dev` on Debian and Ubuntu).
- **Windows:** the generated app doesn't register runtime plugins yet, so calls return `Unavailable`. A custom host can register the plugin itself; it plays through WASAPI.
- **The terminal, HarmonyOS (experimental), and Wear OS / watchOS (experimental):** there is no sound output; calls return `Unavailable`.

## Where your sound files go

`Source::Asset("music/hunt.ogg")` names a file in your app's `assets/` folder. The browser build serves that folder, and `leptoskit build` copies it into every native app (see [images and other assets](36-assets.md)). Or embed a sound in your app with `Source::Bytes(include_bytes!("../assets/swing.wav"))`, up to 8 MiB.

`Source::Data` and `Source::Cache` read a file you downloaded into the app's data or cache folder.
