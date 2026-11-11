---
"leptoskit_cli": patch
---

`leptoskit dev` rejects an invalid `serve.address` before it asks Cargo about the workspace, including a project that has no manifest yet.
