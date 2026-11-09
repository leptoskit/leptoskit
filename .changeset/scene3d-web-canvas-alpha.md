---
"leptoskit_3d": patch
---

# Transparent scenes show the page behind them in the browser

A scene with `Backdrop::Transparent` drew over solid black in the browser. wgpu lists only `Opaque` as a WebGPU canvas's alpha mode, though every browser canvas also takes `premultiplied`, so the canvas was set up opaque. Browser canvases now composite premultiplied, as native surfaces already did. A 3D view over a photo, a camera picture, or another 3D view shows them through its empty pixels. WebGL2, where wgpu refuses anything but opaque, is unchanged.
