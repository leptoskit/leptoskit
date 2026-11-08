---
"release": minor
---

# GPU 3D scenes

The new `leptoskit_3d` crate, also `leptoskit`'s `3d` feature, draws 3D scenes on the GPU in a new `Scene3d` view: glTF models with skins, morph targets, and animation, lit by a directional light and a soft studio environment, each with a soft shadow beneath it. You build a scene with `scene3d::client::Scene` (load a model by path, spawn it, play or crossfade a clip, set morph weights and material overrides, show or hide named parts, attach one model to another's part, place the camera, and raycast), and the host animates it on the display's own clock. Your interface never re-renders to move a model, and a still scene draws nothing.

Scenes draw with WebGPU or WebGL2 in the browser, Metal in the generated iOS and macOS apps, and OpenGL ES in the generated Android apps, where `dev.leptoskit.scene3d.backend` manifest metadata chooses Vulkan instead. `leptoskit build` bundles the app's `assets/` folder into native apps that use 3D. Models may be compressed with meshopt and quantized, as `gltfpack` writes them. Windows and Linux leave an empty space for the view for now, the terminal draws a placeholder, HarmonyOS stops with an error, and the watch surfaces refuse it.

`leptoskit lint` knows `Scene3d` as a built-in component, so its `markup_style` rule holds a scene to the project's `[lint] markup` style like any other view.

The renderer also draws into any wgpu texture you hand it, from one view or several, over a camera image, and hidden behind real-world depth, for hosts that bring their own surfaces, such as augmented and virtual reality.
