---
"leptoskit_cli": patch
---

# Web-only lint skips unused hosts; db/auth App drops unused import

`leptoskit lint`'s clippy pass excludes workspace host crates whose platforms are not listed in `app.platforms`, so setting `platforms = ["web"]` is enough for a green deny-warnings run without GTK or native prepare. `--db` / `--auth` scaffolds no longer put an unused `use leptos::prelude::*` in `App()`. The skill documents the platforms-based clippy skip.
