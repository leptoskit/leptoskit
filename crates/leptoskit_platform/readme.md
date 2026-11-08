# leptoskit_platform

Shared contracts for **platform extension** crates.

First-party destinations such as web, iOS, and macOS stay in `leptoskit_config::Platform`. Unsupported or experimental targets — HarmonyOS, Meta Quest / Oculus, and future vendor shells — live in separate crates that implement [`PlatformExtension`](https://docs.rs/leptoskit_platform) and own their host templates.

The CLI depends on in-tree extension crates today. The same manifest shape is what an out-of-tree sidecar will speak later.

See the design notes in the leptoskit agent store (`docs/platform-extensions.md`) and the HarmonyOS / Oculus crates in this workspace.
