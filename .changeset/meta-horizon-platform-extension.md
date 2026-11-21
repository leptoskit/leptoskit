---
"leptoskit_cli": minor
"leptoskit_platform_horizon": minor
---

# Meta Horizon platform extension and MHCP packager

Add the experimental `leptoskit_platform_horizon` platform extension (`horizon`), registered under `leptoskit native platform` alongside `harmonyos` and `oculus`.

- **`leptoskit native platform scaffold horizon`** generates `hosts/horizon/` with Meta Horizon Worlds Desktop Editor TypeScript Custom UI host scripts (`scripts/scene.ts`, `scripts/LeptosKitHost.ts`, `scripts/scene_test.ts`) implementing leptoskit `version: 1` render transactions over `horizon/ui`, plus Meta Horizon Creator Program (MHCP) offline web submission templates (`mhcp/index.html`, `mhcp/BUILD_LOG.md`).
- **`leptoskit native platform package horizon`** builds and validates a deterministic offline HTML5/WebGL/Wasm `.zip` archive for MHCP hackathon submissions: rewrites root-relative asset references in `index.html` to `./...`, omits `.br`/`.gz` server sidecars, rejects external CDN/network URLs and missing relative assets, and enforces the configurable `--max-mb` size cap (default 35 MiB).
