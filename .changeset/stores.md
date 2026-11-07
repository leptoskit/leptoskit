---
"release": minor
---

# Stores for a state struct

`leptoskit::store` re-exports Leptos's `reactive_stores`: derive `Store` on a state struct and each field becomes its own signal, so reading `store.title()` subscribes a screen to that field alone and writing it never notifies a sibling. The struct stays a plain Rust value, `ArcStore` holds it for as long as a clone lives (like `ArcRwSignal`), and `Store::new` follows the reactive owner instead (like `RwSignal`). Deriving `Patch` as well adds `.patch(next)`, which applies a whole new value — a reducer's output, or fresh data from the server — and notifies only the fields that changed. The derives need `reactive_stores` in the app's `Cargo.toml`; the types come re-exported, at the version leptoskit was built against. Fields are signals like any other, so the same code runs on web, iOS, Android, macOS, Windows, Linux, the terminal, and HarmonyOS (experimental).

`leptoskit::prelude` also gains the `GetUntracked`, `Update`, `With`, and `WithUntracked` signal traits, next to the `Get` and `Set` it already carried, for reading and changing store fields and signals alike.

The `opencode` example keeps its reducer and now applies it with `patch`, and `commonplace` writes one field at a time; both notify per field on every platform.
