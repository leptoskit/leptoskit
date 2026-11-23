---
"leptoskit_native": patch
"leptoskit_wasm": patch
"leptoskit_cli": patch
---

# Binary LKW1 guest wire, smaller guests, and fast Tailwind

- Stream native guest trees to the host with a compact binary wire protocol (`LKW1`) into a reused staging arena instead of per-node JSON serialization, while keeping single-pass JSON decoding for legacy and JS guests.
- Strip dead browser `wasm-bindgen` custom sections and unreachable exports before `wasm-opt -Oz` in `leptoskit_cli`'s guest optimizer, reducing native guest `.wasm` sizes by ~45–52% raw (~29–35% brotli).
- Defer `For`/`Show` region evaluation until expansion (avoiding duplicate subtree construction) and replace heavy sorting networks in `Tailwind::parse` with a compact insertion sort and bitflag fast paths.
