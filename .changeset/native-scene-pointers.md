---
leptoskit_3d: minor
leptoskit_bridge_ffi: patch
---

Deliver primary-pointer drags, taps and cancellations from Android, iOS and macOS scene views through `Scene::next_pointer`. Movement uses layout units and a bounded queue coalesces drags independently of model-loading events.
