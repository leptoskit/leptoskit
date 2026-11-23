---
"leptoskit_yoga": patch
"leptoskit_native": patch
"leptoskit_bridge_ffi": patch
---

# Compact layouts, fast Yoga, streaming diff & O(log N) hit

- Box cold `Style.grid`, `Element.gesture`, and `Element.bindings` fields, pack `Tailwind` into a single `Option<Arc<TailwindInner>>`, gate DevTools-only `#[track_caller]` behind `devtools`, and fuse `Element::validate()` into a single pass.
- Optimize `leptoskit_yoga` with contiguous flex-line ranges, empty/measured leaf fast paths, direct in-flow child iteration, inline `LayoutCache`, and `subtree_pass` dirty pruning.
- Stream `NativeSurface` reconciliation directly without cloning a resolved intermediate tree, prune clean subtrees in `collect_frames`/`place`, binary-search monotonic hit-test siblings in O(log N), switch `Flattener` to a dense zero-clone arena, and encode FFI mutation props by reference without cloning `NativeProps`.
