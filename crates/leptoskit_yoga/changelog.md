# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Fixes

#### Compact layouts, fast Yoga, streaming diff & O(log N) hit

_Owner:_ Ifiok Jr. · _Introduced in:_ [3677b62](https://github.com/leptoskit/leptoskit/commit/3677b6245db20cc2387b3f547855364c319db735)

- Box cold `Style.grid`, `Element.gesture`, and `Element.bindings` fields, pack `Tailwind` into a single `Option<Arc<TailwindInner>>`, gate DevTools-only `#[track_caller]` behind `devtools`, and fuse `Element::validate()` into a single pass.
- Optimize `leptoskit_yoga` with contiguous flex-line ranges, empty/measured leaf fast paths, direct in-flow child iteration, inline `LayoutCache`, and `subtree_pass` dirty pruning.
- Stream `NativeSurface` reconciliation directly without cloning a resolved intermediate tree, prune clean subtrees in `collect_frames`/`place`, binary-search monotonic hit-test siblings in O(log N), switch `Flattener` to a dense zero-clone arena, and encode FFI mutation props by reference without cloning `NativeProps`.

#### Faster Wasm host, external measure & in-place guest tree

_Owner:_ Ifiok Jr. · _Introduced in:_ [53e3f0e](https://github.com/leptoskit/leptoskit/commit/53e3f0e757532c73ef87282497e75c28e7df0fde)

- Consolidate `leptoskit_wasm`'s `HostState` into a single `FastHash` node map with dense `wire_by_node` lookups, bool-deduped dirty lists, cached `TypedFunc` Wasm exports, and O(1) tail detachment in `HostState::detach`.
- Add `Yoga::set_measured` and `Yoga::calculate_layout*_with_measure` so `NativeSurface` measures text via an inline `Copy` `TextMeasureCache` reading `&shadow.props` directly instead of allocating per-node `Arc<RwLock<_>>` closures and cloning strings.
- Add `NativeBackend::apply_owned` so `HeadlessBackend` moves `MutationList` without cloning every `NativeProps`, share a static default `ApproxTextMeasurer`, and accelerate `LogBudget::admit` with exact fast-path JSON byte accounting.
- Reconcile `guest::tree::Tree::update` in place with a leaf fast path, immediate subtree eviction reusing a single DFS stack, and reverse build-order deletions (leaves and trailing siblings first).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_yoga: flexbox and CSS grid layout

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_yoga` is a from-scratch layout engine that turns one `Style` into flexbox or CSS grid geometry, with no C dependencies and no platform lock-in.

`leptoskit_native` calls it for every commit; backends only apply the frames it returns. It is also usable on its own when a Rust tool needs React Native-style layout or a CSS grid implementation without a browser.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
- **Blink f32 leftover-carry for flexible tracks.** Flexible grid tracks now distribute free space with binary32 flex shares, flooring each share to a 1/64 CSS-pixel layout unit and carrying the truncated fraction into the next track—matching Chrome’s `getBoundingClientRect` for equal and weighted `fr` rows/columns. Non-integral percentages such as `w-1/3` floor into a layout unit; whole-number percentages stay exact. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #541](https://github.com/leptoskit/leptoskit/pull/541)
- **Yoga LayoutUnit rounding for Chrome grid parity.** Fractional grid tracks and percentage lengths resolve through Blink-style 1/64 CSS-pixel layout units, so Yoga frames match Chrome `getBoundingClientRect` for the shared Tailwind class trees. The grid parity probe covers nested grids, dense auto-flow, minmax, flex edges, absolute insets, percentages, and overflow. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #540](https://github.com/leptoskit/leptoskit/pull/540)
