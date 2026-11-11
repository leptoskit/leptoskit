---
"leptoskit_bridge_ffi": patch
---

# Composite Android 3D scenes with the interface

Android `Scene3d` views draw into transparent `TextureView` surfaces in the normal view hierarchy. Page backgrounds no longer cover the models, and controls remain above the scene and its camera picture. Changing between the camera and a transparent backdrop no longer removes and recreates the view's GPU surface. Rust releases the native window before Android releases its surface texture.
