# leptoskit_platform_oculus

Experimental **Meta Quest / Oculus** platform extension for leptoskit.

Day-one render strategy is **foreign-headless**: a Wasm guest records mutations; a future Quest shell applies render transactions to a 2D panel (and later world-space quads). The extension id is `oculus`.

Shipping headset APKs that reuse the Android host are the first-party `quest` destination (`leptoskit build --platform quest`, `[native.quest]`). This crate does **not** add an `oculus` variant to `leptoskit_config::Platform`. Scaffold stubs with `leptoskit native platform scaffold oculus`.

```rust
use leptoskit_platform::PlatformExtension;
use leptoskit_platform_oculus::Oculus;

assert_eq!(Oculus.id(), "oculus");
assert!(Oculus.manifest().experimental);
```
