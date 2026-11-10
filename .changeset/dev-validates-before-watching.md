---
"leptoskit_cli": patch
---

# Validate the dev address before watching the workspace

`leptoskit dev` rejects an invalid web serve address before inspecting Cargo metadata, starting a database, or watching files. Watching still begins before the initial build, so edits made during that build trigger a rebuild.
