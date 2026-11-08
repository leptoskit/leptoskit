<h1 align="center">leptoskit_plugin_audio</h1>

<p align="center">
  <strong>leptoskit_plugin_audio</strong> is the leptoskit plugin for sound: looping music, low-latency effects, fades, volume ramps and gliding filters.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_audio"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_audio"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_audio"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_audio.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_audio"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_audio/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Load a sound once from the app's assets, its data or cache folder, or from bytes, and play it as many voices as you need. A voice can loop without a gap, fade in, ramp its volume, pan, pass through two resonant low-pass or high-pass filters whose cutoff glides, pause, and stop with a fade. The master bus has the same volume, filters and pause, so a soundtrack can be veiled, held while the app is in the background, or silenced at once. A crossfade is one voice fading in while another fades out.

Native hosts share one Rust engine. [symphonia](https://docs.rs/symphonia) decodes Ogg Vorbis, WAV, MP3, FLAC and AAC (in MP4). Long music streams: a decoder thread keeps each streaming voice a second ahead, so a ten-minute piece never sits in memory decoded. Short effects are decoded once into memory and shared by every voice of them. The mixer runs in the device's audio callback, takes its commands over a lock-free ring buffer, and never allocates, frees, locks or waits there. It resamples each voice to the device's rate with cubic interpolation and plays through [cpal](https://docs.rs/cpal): Core Audio on iOS and macOS, AAudio on Android, WASAPI on Windows and ALSA on Linux. Filters are state-variable two-pole filters, the response of a biquad, recomputed every sample while they glide so nothing zippers; every change, even an instant one, takes at least five milliseconds so nothing clicks.

Browser apps use `web::Audio` over Web Audio: `AudioBuffer` sources for effects, an `<audio>` element for streams, `BiquadFilterNode`s and `GainNode` ramps. Browsers hold audio back until the page has had a user gesture; the context resumes itself on the first tap or key press.

The audio session is the app's to choose with `configure`. On iOS, `Category::Ambient` mixes with other apps and is silenced by the Ring/Silent switch; `Category::Playback` plays through it and, unless asked to mix, interrupts others. On Android, playback requests audio focus, so the system ducks the app under navigation prompts and other transient sounds, and ambient sound requests none. iOS interruptions arrive as events, and cpal restarts the stream when they end. Android focus loss is not reported yet: hold the master bus when the app goes to the background.

Rust plugins cannot push events to the guest yet, so a voice that plays to its end, and an interruption, are queued and read with `events()`, typically once per frame. At most 64 voices play at once; more return `Busy`. A sound decoded into memory holds at most 16 Mi samples (about three minutes of stereo at 48 kHz); longer ones must stream.

`leptoskit build` copies the app's `assets/` folder into every native app, and `Source::Asset` reads that copy: the `assets` folder beside the executable (iOS: the app bundle's, macOS: `Contents/Resources/assets`) or, on Android, `assets/` inside the APK's assets. HarmonyOS has no output yet and reports `Unavailable`.

Verification covers the wire format, the mixer and filters rendered offline into buffers (a 500 Hz low-pass takes a 5 kHz tone down by 40 dB, ramps and loops have no steps larger than the signal's own, pauses resume where they stopped), decoding of every format from small committed fixtures, streaming across loop points, and the plugin's requests end to end against an offline mixer. The crate builds for iOS, Android and the browser. Playback on a real device was checked on macOS only.

## How it fits

<!-- {=p_audioSummary} -->

Add `leptoskit_plugin_audio` as a direct dependency and select the `audio` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it too; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Audio` rather than resolving the plugin on every call. iOS, Android, macOS, and Linux play through one Rust engine that decodes with symphonia and outputs with cpal; the browser uses `web::Audio` over Web Audio. HarmonyOS has no audio output yet.

<!-- {/p_audioSummary} -->

## Use it

```rust,ignore
use std::time::Duration;
use leptoskit_plugin_audio::{guest::Audio, Filter, LoadMode, Play, Source, Target};

let audio = Audio::new()?;
let hunt = audio.load(Source::Asset("music/hunt.ogg"), LoadMode::Stream).await?;
let music = audio.play(hunt.sound, Play::new().looping(true).fade_in(Duration::from_secs(2))).await?;

// Veil everything: the cutoff glides down over 400 ms.
audio.set_filter(Target::Master, 0, Some(Filter::low_pass(600.0)), Duration::from_millis(400)).await?;

// The app goes to the background.
audio.pause(Target::Master, true, Duration::from_millis(150)).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_audio)
- [Playing sound](https://leptoskit.dev/docs/guide/34-audio.html)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
