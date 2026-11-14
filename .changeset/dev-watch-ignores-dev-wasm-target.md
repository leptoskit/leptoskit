---
"release": patch
---

# `leptoskit dev` stops rebuilding on its own build output

The dev watcher now ignores everything under the project's `target/` directory and resolves the paths it matches against — through symlinks and `..` — to the form filesystem events actually use. Before, the wasm client's own development target directory (`target/leptoskit-dev-wasm/`, which a workspace member's cargo target directory doesn't cover) was watched like source, so every rebuild rewrote `analytics_app.wasm` under it, woke the watcher, and started the next rebuild: `lk dev` on an app with a service plugin rebuilt in a loop with no edits at all. Edits to sources, manifests, and `leptoskit.toml` still rebuild, and apps whose target directory is a symlink (or chosen relative to the workspace) no longer depend on the reported path matching the one events carry.
