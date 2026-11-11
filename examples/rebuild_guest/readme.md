# rebuild_guest

A compiled Wasm guest (package `rebuild_guest_probe`) that proves a screen keeps what it remembers while the native runtime rebuilds it, and lets it go when the screen leaves.

The counter screen reads two counts in its body, so every press rebuilds the whole screen. One count comes from `remember`, the other from `RwSignal::new` in the screen body. A second screen stands in for another route. It exports the same entry points a shipped bundle does (`leptoskit_main`, `leptoskit_event`, `leptoskit_alloc`, `leptoskit_abi_version`).

It is not an app and has no host target: the crate is empty outside `wasm32`. The ignored test `remembered_state_survives_a_rebuild_and_leaves_with_its_screen` in `crates/leptoskit_wasm/tests/rebuild_guest.rs` loads the built `.wasm`, presses Tap twice and checks the remembered count reads 1 then 2 while the fresh count stays 0, then leaves and comes back and checks the remembered count starts over.

```sh
devenv shell test:plugins:guest
```

That task builds this crate for `wasm32-unknown-unknown`, then runs the test with `LEPTOSKIT_REBUILD_GUEST` pointing at the result.
