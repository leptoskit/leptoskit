---
"leptoskit_cli": patch
---

# Scaffold uses shared Health; skill documents web-only trim

New apps re-export `Health` from the shared crate in the app library so `leptoskit lint --deny-warnings` no longer fails on an unused `*_shared` dependency before any RPC types are used. The coding-agent skill says there is no frontend-only `new` flag: after scaffolding, set `platforms = ["web"]` and optionally trim unused host workspace members.
