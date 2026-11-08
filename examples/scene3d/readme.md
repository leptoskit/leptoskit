# scene3d

Turntable, a skinned character swaying as it turns on a stand, and the worked example for 3D scenes.

The screen is one `Scene3d` view between a title and a caption. The scene behind it is built once with ops: load `assets/models/character.glb`, spawn it, play its looping `spin` clip, and place the camera and the light. From then on the host animates the character on the display's clock; the screen never re-renders to move it.

```bash
leptoskit dev                   # in the browser, with WebGPU or WebGL2
leptoskit build --platform ios  # the generated iOS app draws it with Metal
cargo test -p scene3d_example   # the screen validates, the model loads, a frame draws
```

The model is a plain skinned figure, a column bent by eight joints: `leptoskit_3d`'s test character, written by `cargo run -p leptoskit_3d --example character -- assets/models/character.glb 8`.
