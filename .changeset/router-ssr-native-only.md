---
leptoskit_router: patch
---

Keep the file router's SSR module and Axum dependencies on native targets, so enabling all features in browser and Wasm guest builds does not pull in native networking.
