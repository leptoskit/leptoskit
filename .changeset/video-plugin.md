---
"leptoskit_plugin_video": minor
"leptoskit_native": minor
---

# Video players and Video surface

`leptoskit_plugin_video` is a new runtime plugin, `video`, for host-owned URL video players. Create or replace a source, play, pause, seek, set muted/loop/rate, snapshot status, and release the player. Show the same player with a new `Video(player_id)` native widget kind that binds a host-owned id the way `Scene3d` binds a scene (requires an accessible label or `aria_hidden`).

iOS and macOS use `AVPlayer` with an `AVPlayerLayer` surface; Android uses Media3 ExoPlayer with a `PlayerView`; the browser uses an HTML `<video>` element. Windows, Linux, and HarmonyOS return `Unavailable`. Tracks, caching, AirPlay, fullscreen controls, and iOS picture-in-picture enter against an attached player remain out of scope for this release.
