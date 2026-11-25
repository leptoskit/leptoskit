# 3D scenes

Use a 3D scene when your app shows models that move: a character that walks and waves, a product turning on a stand, creatures standing in a room seen through the camera. A `Scene3d` view shows a scene of glTF models, their animations, a light, and a camera, drawn on the GPU.

You describe the scene once, with ops: load a model, place it, play a clip. From then on the app's host animates it on the display's own clock. Your interface code never re-renders to move a model, and a scene that is still costs nothing.

Add `3d` to your app crate's `leptoskit` features, put your models in the app's `assets/` folder, and show a scene:

```rust
use leptoskit::prelude::*;
use leptoskit::scene3d::Light;
use leptoskit::scene3d::Op;
use leptoskit::scene3d::Play;
use leptoskit::scene3d::SceneId;
use leptoskit::scene3d::Transform;
use leptoskit::scene3d::client::Scene;

const TURNTABLE: SceneId = SceneId(1);

#[component]
fn Turntable() -> Element {
	view! {
		<View class="flex-col gap-4 p-6 w-full h-full">
			<Text class="text-2xl font-bold">"Turntable"</Text>
			<Scene3d
				scene=TURNTABLE
				aria_label="A character turning on a stand"
				class="w-full flex-1 rounded-2xl"
			/>
		</View>
	}
}

async fn start_turntable() -> Result<(), leptoskit::scene3d::client::ClientError> {
	let scene = Scene::new(TURNTABLE);
	let model = scene.load("models/character.glb");
	let character = scene.spawn(model);
	scene.play(character, Play::new("spin"));
	scene.push(Op::SetCamera {
		transform: Transform::from_translation(0.0, 1.1, 3.6),
		fov_y: 0.75,
		near: 0.1,
		far: 50.0,
	});
	scene.push(Op::SetLight(Light::default()));
	scene.flush().await
}
```

`Scene3d` takes the scene's id. Give it a size with classes, like any `View`, and either an accessible label (assistive technology reads it as an image) or `aria_hidden=true` when it is decoration. `Scene` queues ops and hands back the ids it chose; `flush` sends them. The repository's `examples/scene3d` is this app, with its model.

## Changing a scene

Everything that changes a scene is an op, sent with the next `flush`:

| You want to                             | Call                                                                                        |
| --------------------------------------- | ------------------------------------------------------------------------------------------- |
| place, scale, or turn a model           | `set_transform(node, Transform::from_translation(x, y, z).with_yaw(radians))`               |
| glide a model somewhere                 | `ease_transform(node, Transform::from_translation(x, y, z), 0.4, Easing::Smooth)`           |
| play, crossfade, or stop a clip         | `play(node, Play::new("walk").fade(0.2))`, `.once()`, `.speed(1.5)`, `stop(node, fade)`     |
| ease a crossfade, or start a clip later | `play(node, Play::new("walk").fade(0.3).fade_easing(Easing::Smooth).phase(0.5))`            |
| show one of a model's variants          | `set_part_visible(node, "head_1", true)`, by the part's node name in the file               |
| change a face or a robe's shape         | `set_morph_weight(node, "smile", 1.0)`, by the morph target's name                          |
| turn a head or swing a leg yourself     | `set_part_offset(node, "head", Transform::IDENTITY.with_yaw(0.4), 0.1)`, by the part's name |
| tint, light up, fade, or outline a part | `set_material(node, Some("robe"), MaterialOverride { … })` (colour, emissive, opacity, rim) |
| make a part glow like light             | `set_blend(node, Some("flash"), Blend::Additive)`                                           |
| draw the scene by hand, in ink          | `set_style(Style::hand_drawn())` or `set_style(Style::print())`                             |
| override relief-print shading           | `set_print(node, PrintTarget::Material("cloth".into()), PrintOverride { … })`               |
| weigh, colour, or drop a part's line    | `set_ink(node, Some("cape"), PartInk { weight: Some(0.0), color: None })`                   |
| pin the directional light to the camera | `set_light(Light { camera_space: true, ..Light::default() })`                               |
| put a sword in a hand                   | `attach(sword, character, "socket_weapon")`                                                 |
| ask what is under a tap                 | `raycast_screen(x, y)`, answered by an `Event::RaycastHits`                                 |
| remove a model                          | `remove(node)`                                                                              |

Ops that name a part, a clip, or a morph target apply even when they arrive before the model has loaded, and ops for names a model does not have do nothing. A clip played with `once()` holds its last pose; the others loop. A new clip crossfades from the one playing; clips don't layer on top of each other. A crossfade blends at a steady rate unless `fade_easing` eases it, and `phase` starts a clip part of the way in, from 0 to 1, so a crowd playing one clip doesn't move in step. `ease_transform` moves a node from wherever it is, and the scene draws every frame of the way; another ease, or a `set_transform`, takes over from where it has got to.

`set_part_offset` moves one named part of a model, a bone or any node, from where its rest pose or the playing clip puts it: the offset turns the part about its own origin and moves it along its own axes, and everything attached below it follows. Use it for what your app decides as it runs, such as a creature turning its head towards the camera while its walk clip plays, or a drum turning one chamber on each shot. The last argument is how many seconds the part takes to get there, in a straight line from where it is now, on the host's own clock. An app that works out a pose on every tick of its own clock and sends it with the tick's length is drawn smoothly between ticks, at the display's rate. Zero moves it at once; `Transform::IDENTITY` puts it back.

`Blend::Additive` adds a part's colour to whatever is behind it instead of covering it, scaled by its opacity: a muzzle flash, a spark, or a halo stays bright over a camera picture of a dark room, and black adds nothing, so a soft round glow needs no cut-out. Light added to something already bright barely shows, so for a glow that must read over a white wall, put a darker, normally blended halo behind it. Additive parts draw with the transparent parts and hide nothing behind them.

## A hand-drawn look

`set_style(Style::hand_drawn())` draws the whole scene like an inked drawing: the light falls in a few flat bands instead of a smooth gradient, and every model gets an ink line around it. `Style::default()` is the physically based look, and a scene that never sets a style draws exactly as it did before styles existed.

```rust
scene.set_style(Style {
	shading: Shading::Toon(Toon { bands: 3, shade: [0.42, 0.40, 0.58], ..Toon::default() }),
	ink: Some(Ink { width: 2.0, facing: 0.6, shade_weight: 0.8, ..Ink::default() }),
});
```

- **`Toon`** shades in 2 to 4 `bands`, with antialiased edges a pixel wide that `softness` (0 to 1) widens. The darkest band takes the `shade` tint rather than going black, the ambient light is flat, the highlight is a hard spot (`specular`, 0 for none), and a part's rim (`MaterialOverride::rim`) gets a hard edge. `hatch` (0 to 1, off by default) draws ink hatching in the darkest band, fixed to the surface at 60 lines to a metre of the model, fading out where the lines would crowd closer than a few pixels.
- **`Ink`** draws its line in `color`, exactly, `width` thousandths of the view's height wide (2 is a firm pen line), so it looks the same on every screen and in each eye of a headset, and does not thin as a model walks away. `wobble` (0 to 1) makes the width wander like a pen's; the wander is fixed to the model's surface, so it never crawls. `boil` draws the wander afresh that many times a second, in steps, and only on frames the scene draws anyway; `seed` picks its pattern, and the same scene time always draws the same line. `shade_weight` makes the line heavier on the side away from the light, and `facing` adds a line inside a model where its surface turns away from you.
- **`set_ink`** changes one part's line, with everything under it: `weight` multiplies the width (0 for none, up to 8; at 1 and above the outline keeps a 1-pixel floor, while weights below 1 taper that floor to 0 so a line can fade out smoothly) and `color` replaces the ink's. A field left `None` keeps what a setting on an enclosing part gave it, so `PartInk::default()` returns a part to what encloses it. Unlit parts have no line unless one is set; parts too small on screen for a line to read get none.
- **A model can weigh its own line**, per vertex, in the red channel of its `COLOR_1` attribute: 0.5 is the part's line, 0 none, and 1 twice as heavy. Green, blue, and alpha of `COLOR_1` are reserved for app-specific masks and are ignored by the renderer. A model without `COLOR_1` has the part's line everywhere. gltfpack keeps `COLOR_1` as written.
- **Relief-print shading (`Style::print()` / `Shading::Print`)** skips PBR textures and tonemapping to output exact linear colours in three bands (`lit`, `shade`, `deep`, their edges blended over about a pixel) or flat colour (`PrintMode::Flat`, the default for unlit materials). Use `set_print` (`Op::SetPrint`) to assign palettes, hatching, turning-edge paper specular, backlit spot rim, line weight, knockout participation, and per-figure 3-slot boil (`boil_step % 3`) by `PrintTarget::All`, `PrintTarget::Material` (matching the base name before `.`), or `PrintTarget::Part` (highest precedence). Setting `Print::knockout_width > 0` draws a single-pass stencil paper knockout (`Depth24PlusStencil8`) around each figure outside its key line: overlapping figures separate cleanly, models joined with `attach` (`Op::Attach`, such as held weapons) share their figure's silhouette with no interior seam while a model spawned under another (a figure on a stage) keeps its own, and open single-sided sheets frame along their open edges. Styles without a knockout stay on `Depth32Float`.
- **Camera-space lighting & smoothed outline hulls:** setting `Light::camera_space = true` rotates the directional light into each view's camera space on the host so rapid camera motion stays locked to the light. When loading faceted models directly, `Asset::from_slice_with_options` (`LoadOptions::outline_smooth_rings`) or glTF `extras.outlineSmoothRings` (0 to 8; default 0) optionally runs vertex-adjacency Laplacian smoothing on welded outline normals.
- **Wire compatibility:** relief-print styles (`Shading::Print`), `Op::SetPrint`, and `Light::camera_space = true` use additive v1 op tags (`27..=29`) and require a matching host version; scenes that do not use them remain byte-for-byte compatible with older v1 hosts.

The line is a slightly larger copy of each model drawn behind it, so it follows skins, morph targets, and animation, and real-world depth hides it like the model. It outlines closed shapes: an open sheet, such as a robe's hem with no thickness, gets no line along its open edge, so give such edges a little thickness (Blender's Solidify modifier) to ink them. Alpha-masked (cutout), blended, faded, and additive parts never draw the outline, even with a weight set (they still keep inner lines and hatching where visible); to fade a model out, fade its line out with `set_ink` first.

`next_events` waits for what the host has to say: a model loaded (with its clips' names) or failed to, a clip played once has finished, or what a raycast hit, nearest first, with the part's name, the distance, and the point.

## Models

Scenes load glTF 2.0: `.glb` files, or `.gltf` files with their buffers and images beside them or embedded. Meshes may be compressed with meshopt (`EXT_meshopt_compression`) and quantized (`KHR_mesh_quantization`), with texture coordinates scaled and offset by `KHR_texture_transform`, as `gltfpack` writes them. A material's textures share one transform: the first one found, starting from its base color texture. Skins, morph targets, and animations of transforms and morph weights (step, linear, and cubic spline) play; metallic-roughness, unlit, and emissive materials draw, with opaque, masked, and blended alpha; textures are PNG or JPEG. A file that needs any other required extension, such as Draco, fails to load with an event that says which.

A model loaded by several scenes, or spawned many times, is read and uploaded to the GPU once. Identical static parts draw together in one call.

## How it draws

Each frame lights the scene with one directional light and a soft studio environment, puts a soft shadow under each model, and tonemaps the result so bright highlights roll off rather than clip. Ink lines draw in their own colour, untouched by exposure and tonemapping, in the same pass as the models. Edges are smoothed with 4× multisampling. Where the view is transparent, what is behind it shows through.

`Op::SetBackdrop` chooses what is behind the models: nothing (`Backdrop::Transparent`, the default), a colour, or `Backdrop::Camera`. On iOS and Android the camera backdrop is the live camera picture while an [`xr` session](34-augmented-reality.md#draw-over-the-camera) runs, drawn from the session's viewpoint; without one it is transparent.

The scene draws only while something changes or animates: a scene whose clips have finished, or that plays nothing, stops asking for frames until the next op.

## Platforms

- **Web:** the view is a `<canvas>`, drawn with WebGPU, or WebGL2 where the browser has no WebGPU. Models load from `/assets/`, where `leptoskit build` copies your `assets/` folder; in the browser only `.glb` files and `.gltf` files with embedded data load. With WebGL2, a scene shown in two views at once draws only in the first.
- **iOS and macOS:** the view draws with Metal on the display's refresh. `leptoskit build` bundles your `assets/` folder into the app.
- **Android:** the view draws with OpenGL ES by default, because some Vulkan drivers have reset the GPU when the camera closes. Apps that have tested their devices can choose Vulkan with `<meta-data android:name="dev.leptoskit.scene3d.backend" android:value="vulkan" />` in the application's manifest. `leptoskit build` packs your `assets/` folder into the APK.
- **Windows and Linux:** not drawn yet. The apps leave an empty space the size of the view.
- **The terminal:** draws a placeholder in the view's cells, as it does for an `Image`.
- **HarmonyOS (experimental):** not supported: like the other components its host doesn't draw, a `Scene3d` stops with an error.
- **Wear OS and watchOS (experimental):** watch surfaces refuse the view.

Native apps draw scenes in their own host, never in the app's Wasm code, so a busy screen does not slow a scene down.

## Limits

A scene may hold 4,096 nodes and refer to 64 models; an app may hold 16 scenes. A model may be 128 MiB, 4 million vertices, and 16,384 nodes, with 64 morph targets per mesh, 256 joints per skin, and textures up to 4,096 pixels a side. Numbers in ops are bounded to finite, sensible ranges, and model paths must stay inside the app's assets.

Wire format `VERSION = 1` keeps older batches byte-for-byte compatible, while `set_style`, `set_ink`, `ease_transform`, and `play` with `fade_easing` or `phase` use additive v1 op tags (`23..=26`) that require a matching host/renderer version (an older v1 host rejects any batch containing them).

Not supported yet: ink lines along open edges and around alpha-masked or blended parts, shadows cast onto other models, more than one light, image-based lighting from the camera, textures other than PNG and JPEG, a different `KHR_texture_transform` for each texture of one material, texture coordinates beyond the first set, and stereo through multiview.
