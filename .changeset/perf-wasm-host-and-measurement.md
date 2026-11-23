---
"leptoskit_yoga": patch
"leptoskit_native": patch
"leptoskit_wasm": patch
---

# Faster Wasm host, external measure & in-place guest tree

- Consolidate `leptoskit_wasm`'s `HostState` into a single `FastHash` node map with dense `wire_by_node` lookups, bool-deduped dirty lists, cached `TypedFunc` Wasm exports, and O(1) tail detachment in `HostState::detach`.
- Add `Yoga::set_measured` and `Yoga::calculate_layout*_with_measure` so `NativeSurface` measures text via an inline `Copy` `TextMeasureCache` reading `&shadow.props` directly instead of allocating per-node `Arc<RwLock<_>>` closures and cloning strings.
- Add `NativeBackend::apply_owned` so `HeadlessBackend` moves `MutationList` without cloning every `NativeProps`, share a static default `ApproxTextMeasurer`, and accelerate `LogBudget::admit` with exact fast-path JSON byte accounting.
- Reconcile `guest::tree::Tree::update` in place with a leaf fast path, immediate subtree eviction reusing a single DFS stack, and reverse build-order deletions (leaves and trailing siblings first).
