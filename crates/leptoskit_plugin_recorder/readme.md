<h1 align="center">leptoskit_plugin_recorder</h1>

<p align="center">
  <strong>leptoskit_plugin_recorder</strong> records the microphone continuously in the background as timed AAC-LC mono <code>.m4a</code> segments, for apps that upload and transcribe each segment themselves.
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_recorder"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_recorder"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_recorder.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_recorder"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_recorder/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Continuous background recording for a life-logging app: the phone records ambient audio all day in segments (default 300 seconds, 5–3600 clamped), and the app decides what to do with each one. Segments are AAC-LC, mono, `.m4a`, rotated at each boundary with no gap between segments. The plugin measures each 30-millisecond frame's RMS and peak level and reports a segment's `peak_dbfs` and `active_ms`, so the app can skip uploading quiet segments without decoding them. It never deletes a delivered segment and never decides whether one is silent; the app owns both choices.

| Platform        | Implementation                                          | Scope                                                                  |
| --------------- | ------------------------------------------------------- | ---------------------------------------------------------------------- |
| Android         | Kotlin `AudioRecord` + `MediaCodec`/`MediaMuxer` thread | Continuous segments, permission, silenced reporting, crash recovery    |
| iOS             | Swift `AVAudioEngine` tap + AAC `AVAudioFile`           | Continuous segments, permission, interruption handling, crash recovery |
| macOS           | Unavailable                                             | A Mac app that wants this should record through its own host code      |
| Windows / Linux | Unavailable                                             | Use explicit registration in a custom host if one is added later       |
| Browser         | Unavailable                                             | A hidden tab loses its microphone stream                               |

## Native registration and use

Enable the `guest` feature in Wasm guest code, add `leptoskit_plugin_recorder` as a direct dependency of each native host, and select `recorder` in `native.plugins`. The generated host discovers the crate from Cargo metadata. The guest client caches its numeric plugin ID and reuses `guest::Recorder`.

The CLI merges the plugin's manifest needs into the generated app: `RECORD_AUDIO` in the Android manifest, and `audio` in the iOS `UIBackgroundModes`. The app owns its `NSMicrophoneUsageDescription` purpose string in `ios/Info.plist`; the build fails until it is there, with the app's own words.

Android background recording also needs the `background` plugin: start its foreground service with the `microphone` type (bit 4 of its service-type mask) so the process stays alive and the system lets the microphone keep running. `start` fails with `Unavailable` when the app is neither in the foreground nor running that microphone-typed service. Android 14 cannot start a microphone foreground service from `BOOT_COMPLETED`, so after a reboot the owner must open the app once. When another app needs the microphone, Android silences this app's capture; `status` reports `Silenced` and segments keep rotating with silence.

iOS recording can only start in the foreground (`start` fails there otherwise); the `audio` background mode keeps it running afterwards. The session is `.playAndRecord` with `.mixWithOthers`, `.allowBluetooth` and `.defaultToSpeaker`, so the owner's music and calls keep working. A call or Siri closes the current segment (reported as `ended_by = Interruption`), status reports `Interrupted`, and the plugin resumes in a fresh segment when the system allows it. Route changes rotate the segment and rebuild the input tap, and a media services reset rebuilds the whole engine.

## Crash recovery

A small append-only journal sits beside the segments and records every file the recorder opens, finishes and delivers. After a crash or kill, the next `start` — or a `recover` call — delivers the leftover finalised files as segments with `ended_by = Recovered`, deletes the truncated files that cannot be read, and reports what it found. Delivery is at-least-once: a file delivered just before a crash may be delivered again, so dedupe by path or start time.

## Binary protocol

Integers and floats are little-endian; a string is a `u32` byte length and its UTF-8 bytes, and an absent optional string is the length `u32::MAX`. Every decoder rejects trailing bytes, truncated fields, invalid UTF-8 and out-of-domain values before calling a platform API.

| ID | Method        | Request                                                                           | Response                                                                                                       |
| -- | ------------- | --------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| 0  | Permission    | Empty                                                                             | State `u32`: Unknown=0, Granted=1, Denied=2; never prompts                                                     |
| 1  | Request perm. | Empty                                                                             | State `u32` after the system screen; the only call that can prompt                                             |
| 2  | Start         | Dir string, segment secs `u32`, sample rate `u32`, bitrate `u32`, threshold `f64` | Recovered `u32`, discarded `u32`                                                                               |
| 3  | Stop          | Empty                                                                             | Empty; the current segment is finished and delivered, not lost                                                 |
| 4  | Status        | Empty                                                                             | State `u32`, segment start `i64`, optional reason string                                                       |
| 5  | Next segment  | Empty                                                                             | Present `u32`; when 1: path, started `i64`, duration `i64`, bytes `i64`, peak `f64`, active `i64`, ended `u32` |
| 6  | Recover       | Empty                                                                             | Recovered `u32`, discarded `u32`                                                                               |

Segment lengths are 5–3600 seconds (default 300, clamped), sample rates 8–48 kHz (default 16 kHz), bitrates 8–320 kbps (default 24 kbps), thresholds −100–0 dBFS (default −45). `active_ms` counts 30 ms frames whose RMS stayed above the threshold. A segment ends by Rotation=0, Stop=1, Interruption=2 or Recovered=3. Status is Idle=0, Recording=1, Interrupted=2, Silenced=3 or Failed=4, and only Failed carries a reason.

## How it fits

<!-- {=p_recorderSummary} -->

Add `leptoskit_plugin_recorder` as a direct dependency and select the `recorder` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Recorder` rather than resolving the plugin on every call. iOS records through `AVAudioEngine` and Android through `AudioRecord` with a `MediaCodec` AAC encoder; the app supplies its `NSMicrophoneUsageDescription` purpose string and, for Android background recording, runs the `background` plugin's microphone-typed foreground service.

<!-- {/p_recorderSummary} -->

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_recorder)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
