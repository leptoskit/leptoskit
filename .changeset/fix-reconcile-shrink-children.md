---
"leptoskit_native": patch
---

# Fix panic when shrinking children during reconcile

- Slice added children from `common..new_len` instead of `old_len..new_len` in `NativeSurface::update_source_node` so removing children from an existing container no longer panics with an out-of-range slice index.
