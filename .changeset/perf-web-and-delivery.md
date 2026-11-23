---
"leptoskit_native": patch
"leptoskit_cli": patch
"leptoskit_axum": patch
---

# Smaller web Wasm, lazy listeners & precompressed serving

- Remove `sha2` from `leptoskit_native`'s `web` feature in favor of a deterministic 64-bit `ScopeHasher`, emit non-default base CSS style deltas directly with `styles::delta_for`, fuse tree pre-scans into one `scan_tree` pass, attach DOM event listeners lazily via `Either` only when handled, and make `NodePath` zero-sized without `devtools`.
- Inject `<link rel="modulepreload">` and `<link rel="preload" as="fetch" type="application/wasm" crossorigin>` hints into built `index.html` files and enable `--strip-producers --low-memory-unused` in `leptoskit_cli`'s `wasm-opt` pipeline.
- Negotiate precompressed `.br`/`.gz` `index.html` in `leptoskit_axum`'s `SpaFallback` and serve precompressed static site/docs assets with `Vary: accept-encoding` in `leptoskit_app`.
