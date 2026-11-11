---
"leptoskit_bridge_ffi": patch
"leptoskit_cli": patch
"leptoskit_router_codegen": patch
---

# Native scenes load models on the first launch

Android, iOS and macOS install the scene asset reader before starting the guest. Models queued before the first native scene view exists now load from the app bundle on a cold launch.

Generated Axum route lists also return their owned paths directly, keeping generated apps free of redundant conversion warnings.
