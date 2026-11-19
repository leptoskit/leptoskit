---
"leptoskit_admin": minor
"leptoskit_admin_pages": minor
"leptoskit_cli": minor
---

# The admin's pages start moving to Rust in the browser

The admin's pages now live in a new crate, `leptoskit_admin_pages`, which builds for the server and for Wasm. `leptoskit_admin` renders a page there from a small model, and the browser hydrates the same component tree, so the page's behaviour is Rust instead of the hand-written `admin.mjs`.

The Glossary is the first page to move. Its theme switch, account menu, search, ⌘K (Ctrl+K elsewhere) to find a screen, and Shift+W for Workings mode all run in Wasm. Every other page still uses `admin.mjs` for now.

Build the client with `leptoskit build --release --platform web` in `crates/leptoskit_admin_pages`, and give the admin its output directory with the new `AdminBuilder::client_dir`, or `lk admin serve --client-dir` (also read from `LEPTOSKIT_ADMIN_CLIENT_DIR`). The admin serves it from `/_leptoskit/admin/pkg/`. Without it, moved pages render but don't respond to presses.

`Environment` now comes from `leptoskit_admin_pages` and is re-exported as before. Parsing one fails with `UnknownEnvironment` instead of `AdminError::Config`.
