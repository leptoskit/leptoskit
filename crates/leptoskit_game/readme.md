# leptoskit_game

A game is a leptoskit app with a world behind the interface.

Gameplay stays Rust. You implement `Game`: `setup` builds the world, and `tick` advances it on a fixed step. Menus, HUDs, and buttons are ordinary leptoskit components (`View`, `Text`, `Button`). Each frame the host commits that tree through `NativeSurface`, the same mutation plan the native backends apply, and draws the resulting boxes over a wgpu picture of the world. Nothing here is a second UI language or a second compiler.

The picture is the canvas renderer (Vulkan on Linux, Metal on Apple, Direct3D 12 on Windows). A machine with no adapter gets the same frame from a CPU painter, and the run report says which one drew it.

Apps opt in with the `game` feature on `leptoskit`. The crate is not part of the default build. `leptoskit game run` builds and runs a game crate; the sample is `game/lantern`.
