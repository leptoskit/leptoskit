# leptoskit_platform

Shared contracts for **platform extension** crates.

First-party destinations (web, iOS, Android, macOS, Windows, Linux, terminal, browser extension, and shipping Meta Quest packaging via `quest`) stay in `leptoskit_config::Platform`. Niche or experimental shells that should not enlarge those enums forever implement [`PlatformExtension`](https://docs.rs/leptoskit_platform) and own their host templates and capability manifests.

Today the CLI registers in-tree extensions through a builtin table and exposes them under `leptoskit native platform`. The same [`Manifest`](https://docs.rs/leptoskit_platform) shape is what an out-of-tree sidecar will speak later.
