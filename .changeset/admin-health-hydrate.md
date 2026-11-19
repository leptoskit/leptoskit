---
"leptoskit_admin": patch
"leptoskit_admin_pages": minor
---

# A project's Health tab runs as the pages' own Rust

A project's Health tab is now a page the admin's Wasm client picks up (`Body::Health`), like its other tabs: the health check with its Run button, and the fleet's last hour of host, export and latency charts. The pages crate turns on leptoskit's `apm` feature to draw them.
