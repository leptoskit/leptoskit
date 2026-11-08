---
"leptoskit_cli": minor
---

# New apps use the file router by default

`leptoskit new` enables the `router` plugin, scaffolds `index.rs` / `_layout.rs` / `+not-found.rs` under the app crate's `src/routes/`, commits a generated `routes.rs`, and depends on `leptoskit_router`. Builder `[lint]` stays the default so the starter `Page` and `Layout` components stay clean under `leptoskit lint --deny-warnings`.
