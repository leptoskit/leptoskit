# soundboard

A compiled Wasm guest (package `soundboard_example`) for the [`audio` plugin](../../crates/leptoskit_plugin_audio). Its one screen, written with `view!`, loads a short effect from embedded bytes, plays it when **Strike** is pressed, and counts the voices that played to their end when **Count endings** is pressed.

It is not an app and has no host target: the crate is empty outside `wasm32`. The ignored test `the_soundboard_plays_a_sound_on_a_press_and_reports_its_end` in `crates/leptoskit_plugin_audio/tests/soundboard_guest.rs` loads the built `.wasm` in the interpreter, answers its plugin calls with the real engine running offline, presses both buttons, and checks what the mixer renders and what the screen says.

```sh
devenv shell test:plugins:guest
```
