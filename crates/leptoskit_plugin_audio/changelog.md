# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

#### Sound playback

_Owner:_ Ifiok Jr. · _Introduced in:_ [67ea0f1](https://github.com/leptoskit/leptoskit/commit/67ea0f111c73a3d36f2472c94976a24cf52b7e02)

`leptoskit_plugin_audio` is a new runtime plugin, `audio`, for music and sound effects. Load a sound from the app's assets, its data or cache folder, or bytes, and play it as many voices as you need: looping without a gap, fading in, ramping volume, panning, through two low-pass or high-pass filters whose cutoff glides, paused and resumed where it stopped, and stopped with a fade. The master bus has the same volume, filters, and pause, so a soundtrack can be veiled, held while the app is in the background, or silenced at once, and `crossfade` brings one piece in as another falls away. `events()` reports voices that played to their end and audio-session interruptions.

iOS, Android, and macOS share one Rust engine: symphonia decodes Ogg Vorbis, WAV, MP3, FLAC, and AAC; music streams a second ahead instead of sitting in memory decoded; a lock-free mixer runs in the device callback; and cpal plays through Core Audio and AAudio. `configure` picks an ambient or playback session: the `AVAudioSession` category on iOS, and audio focus on Android. Browser apps use `web::Audio` over Web Audio, which starts on the page's first tap or key press. HarmonyOS has no output yet, Android does not report losing audio focus, and native builds don't copy `assets/` into the app yet.

### Fixes

- **Clarify that buffered browser sounds report the decoded AudioContext sample rate, and verify their length and playback controls at both 44.1 kHz and 48 kHz.** _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
