---
"leptoskit_cli": patch
---

# Generated sign-in screens run their mount effects in native apps

`leptoskit new --auth` and `leptoskit generate auth` now use isomorphic effects for the sign-in screen's mount step, the provider-notice step, and the post-sign-in navigation. `Effect::new` is compiled out in a native guest (the generated app does not enable leptos's `effects` feature), so the screen could stay on "Checking whether you're signed in…" forever. The effects still skip server rendering, so the first server HTML matches the browser's first render.

The generated notes journey is now exercised end to end in the real interpreter: the optimized guest loads and refreshes notes over the HTTP plugin, validates the draft, shows pending/saved/failed saves, signs in through the development mailbox, switches accounts without showing the previous account's rows, and rejects late list and save replies after their screen is disposed.

`leptoskit generate auth` on an existing database app still adds the screens and asks the app to call `auth::provide()` and put `auth::screens().gate(…)` in front of its own home screen. Rewriting an app's root automatically is not safe, so that manual step stays open; the journey covers the fully scaffolded `--auth` app, which is already wired.
