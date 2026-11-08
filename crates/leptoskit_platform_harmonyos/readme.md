# leptoskit_platform_harmonyos

Experimental **HarmonyOS** platform extension for leptoskit.

This crate owns the ArkUI / Stage-model host templates that used to live under `leptoskit_cli`'s `native/templates/harmonyos`. The CLI still orchestrates prepare, NDK cross-compilation, plugin registration, and HAP packaging; templates and the capability [`Manifest`](https://docs.rs/leptoskit_platform) come from here.

HarmonyOS is not a committed first-party shipping target. Prefer it for experiments; do not treat HAP packaging as production-ready without device validation.

```rust
use leptoskit_platform::PlatformExtension;
use leptoskit_platform_harmonyos::HarmonyOs;

let extension = HarmonyOs;
assert_eq!(extension.id(), "harmonyos");
let _ = extension.manifest();
```
