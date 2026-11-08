# leptoskit_platform

Shared contracts for **platform extension** crates.

First-party destinations such as web, iOS, macOS, and Meta Quest (`quest`) stay in `leptoskit_config::Platform`. Unsupported or experimental shells that should not enlarge the core forever — HarmonyOS today, and future vendor targets — live in separate crates that implement [`PlatformExtension`](https://docs.rs/leptoskit_platform) and own their host templates.

The CLI depends on in-tree extension crates today. The same manifest shape is what an out-of-tree sidecar will speak later.

See `leptoskit_platform_harmonyos` in this workspace for the first in-tree extension.
