---
"release": minor
---

# Platform extension contracts

Adds `leptoskit_platform`: versioned manifests, capability matrices, and scaffold file lists for niche OS shells. The CLI exposes a live builtin registry through `leptoskit native platform list|capabilities|scaffold`. Concrete in-tree shells land in follow-up crates; out-of-tree sidecars will reuse the same manifest shape later.
