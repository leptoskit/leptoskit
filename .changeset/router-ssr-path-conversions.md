---
"leptoskit_router_codegen": patch
---

# Generate SSR paths without redundant conversions

Generated `axum_route_list` entries now use the `String` that `Pattern::axum_path` returns directly. The redundant conversion made generated SSR routes fail clippy when warnings were denied.
