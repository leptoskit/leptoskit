---
"leptoskit_service_worker": patch
---

The crate compiles for `wasm32` again: the crate root's `Listener` re-export was illegal on that target, where the item comes from the browser module rather than the native fallback, and an extension-style listener helper nothing called is gone.
