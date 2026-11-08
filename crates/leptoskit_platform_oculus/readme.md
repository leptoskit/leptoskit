# leptoskit_platform_oculus

Experimental **Meta Quest / Oculus** platform extension for leptoskit.

This crate is the in-tree home for Quest host stubs and the capability manifest. Packaging, OpenXR compositor wiring, and store install are not implemented yet — the same experimental bar as early Wear OS hosts.

Day-one render strategy is **foreign-headless**: a Wasm guest records mutations; a future Quest shell applies render transactions to a 2D panel (and later world-space quads). The destination id is `oculus`.

```rust
use leptoskit_platform::PlatformExtension;
use leptoskit_platform_oculus::Oculus;

assert_eq!(Oculus.id(), "oculus");
assert!(Oculus.manifest().experimental);
```
