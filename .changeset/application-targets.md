---
"release": minor
---

Build several frontends, servers and native destinations from one application's root `leptoskit.toml` and Cargo workspace. Named targets select a frontend and explicit server package, isolate public inputs, sidecars, outputs and build records, and preserve application identity. Use `--app-target NAME` and `leptoskit targets`; destinations not yet implemented can carry an actionable unavailability reason. Native hosts can live under `platforms/`. macOS menu-bar mode keeps a status item alive and opens its native UI only when requested.
