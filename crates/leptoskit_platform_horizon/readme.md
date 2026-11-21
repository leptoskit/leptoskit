# leptoskit_platform_horizon

Experimental **Meta Horizon** platform extension for leptoskit.

Supports two Meta Horizon workflows under the `horizon` extension id (`leptoskit native platform`):

1. **Meta Horizon Worlds (`horizon/ui` TypeScript API 2.0):** scaffolds a foreign-headless scene interpreter (`scripts/scene.ts`) and `UIComponent` host (`scripts/LeptosKitHost.ts`) that renders leptoskit render transactions into `horizon/ui` `View`, `Text`, `Pressable`, `ScrollView`, and `Image` nodes.
2. **Meta Horizon Creator Program (MHCP) hackathon web bundles (`.zip`):** validates and packages self-contained, offline HTML5 / WebGL / Wasm builds (`index.html` at the archive root, relative paths, zero external CDN/network URLs, ≤ 35 MiB default budget) via `leptoskit native platform package horizon`.

Shipping Meta Quest (Horizon OS) headset APKs stay on the first-party `quest` destination (`leptoskit build --platform quest`, `leptoskit publish quest`). This crate does **not** add a `horizon` variant to `leptoskit_config::Platform`.

```rust
use leptoskit_platform::PlatformExtension;
use leptoskit_platform_horizon::Horizon;

assert_eq!(Horizon.id(), "horizon");
assert!(Horizon.manifest().experimental);
```
