---
"release": minor
---

# Game host

`leptoskit game` builds and runs a game: gameplay stays Rust (`Game::setup` and `Game::tick` on a fixed step), and menus are ordinary leptoskit components committed through the same native renderer as the other platforms. The world is drawn with wgpu, or on the CPU when the machine has no adapter. The sample is `game/lantern`. Enable it with the `game` feature. The host writes a PNG frame; it does not open a window or package a phone or browser app.
