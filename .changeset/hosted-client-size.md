---
"release": patch
---

The hosted app's wasm clients leave out panic source locations and `Debug` text. Those strings are different in every crate, so brotli barely shrinks them, and they had pushed the public site's initial load over its budget. `remember_keyed` is not used here; it tells call sites apart by those locations.
