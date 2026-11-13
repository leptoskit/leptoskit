# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

#### Add the recorder plugin: continuous microphone segments

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #737](https://github.com/leptoskit/leptoskit/pull/737)

Add a `leptoskit_plugin_recorder` crate with a typed binary guest client, Swift and Kotlin handlers, and automatic native registration under the runtime name `recorder`. It records the microphone continuously in the background as timed audio segments: AAC-LC, mono, `.m4a`, rotated at each boundary with no gap between segments, for apps that upload and transcribe each segment themselves.

`start` takes a directory inside the app's own data or cache directories and a segment length (default 300 s, clamped 5–3600 s), sample rate (default 16 kHz), AAC bitrate (default 24 kbps) and activity threshold (default −45 dBFS). `next_segment` is a cancellable wait that returns each finished segment with its path, start time, duration, size, peak level and `active_ms` — the milliseconds of 30 ms frames above the threshold — so the app can skip uploading quiet segments without decoding them. The plugin never deletes a delivered segment and never decides whether one is silent; the app owns both choices. `status` reports idle, recording, interrupted (a call or Siri), silenced (Android: another app holds the microphone) or failed with a reason, plus the open segment's start time.

A small append-only journal beside the segments survives a crash: the next `start` (or a `recover` call) delivers leftover finalised files as `ended_by = recovered` segments and deletes the truncated files that cannot be read, reporting each deletion once. iOS records through an `AVAudioEngine` input tap into an AAC `AVAudioFile`, configures `.playAndRecord` with `.mixWithOthers`, `.allowBluetooth` and `.defaultToSpeaker` so the owner's music and calls keep working, closes the segment on an interruption and resumes when the system allows it, and rebuilds the tap on route changes and after a media services reset; recording can only start in the foreground, and the `audio` background mode keeps it running. Android records `AudioRecord` PCM into a `MediaCodec` AAC encoder and `MediaMuxer` on its own thread, reports `silenced` through the audio recording callback, and requires the `background` plugin's microphone-typed foreground service for background recording — which Android 14 cannot start from `BOOT_COMPLETED`, so the owner must open the app once after a reboot. The `background` plugin now exposes the running service's type mask so `start` can fail with a clear error instead of recording into a process the system will kill.
