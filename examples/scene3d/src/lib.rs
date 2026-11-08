//! Turntable: a skinned character turning on a stand, the worked example for
//! 3D scenes.
//!
//! The screen holds one `Scene3d` view. The scene behind it is built with
//! ops, once: load the model, spawn it, play its `spin` clip, and place the
//! camera and the light. From then on the host animates the character on
//! its own display link (the browser's `requestAnimationFrame`, or the
//! native app's display link); the screen never re-renders to move it.
//!
//! ```
//! use leptoskit::scene3d::Op;
//! use leptoskit::scene3d::client::Scene;
//! use scene3d_example::SCENE;
//!
//! let scene = Scene::new(SCENE);
//! scene3d_example::build(&scene);
//! let ops = scene.take_ops();
//! assert!(matches!(&ops[0], Op::LoadAsset { path, .. } if path == "models/character.glb"));
//! ```

use leptoskit::prelude::*;
use leptoskit::scene3d::Light;
use leptoskit::scene3d::NodeId;
use leptoskit::scene3d::Op;
use leptoskit::scene3d::Play;
use leptoskit::scene3d::SceneId;
use leptoskit::scene3d::Transform;
use leptoskit::scene3d::client::Scene;

/// The scene the turntable shows.
pub const SCENE: SceneId = SceneId(1);

/// The model, relative to the app's `assets/` folder.
pub const MODEL: &str = "models/character.glb";

/// Queues the ops that build the turntable, and returns the character's
/// node.
pub fn build(scene: &Scene) -> NodeId {
	let model = scene.load(MODEL);
	let character = scene.spawn(model);
	// `spin` loops: the host keeps it playing without the app's help.
	scene.play(character, Play::new("spin").fade(0.3));
	scene.push(Op::SetCamera {
		transform: Transform::from_translation(0.0, 1.1, 3.6),
		fov_y: 0.75,
		near: 0.1,
		far: 50.0,
	});
	scene.push(Op::SetLight(Light::default()));
	character
}

/// The turntable screen: a title, the scene, and a caption.
#[component]
pub fn Turntable() -> Element {
	view! {
		<View class="flex-col gap-4 p-6 w-full h-full bg-stone-100">
			<Text class="text-2xl font-bold text-stone-900">"Turntable"</Text>
			<Scene3d
				scene=SCENE
				aria_label="A character swaying as it turns on a stand"
				class="w-full flex-1 rounded-2xl bg-stone-200"
			/>
			<Text class="text-sm text-stone-600">
				"The host animates the character; this screen never re-renders to move it."
			</Text>
		</View>
	}
}

/// Mounts the turntable when the browser loads the Wasm module, then sends
/// the scene's ops. `leptoskit dev` builds it with the `csr` feature and
/// serves it from `web/index.html`.
#[cfg(all(target_arch = "wasm32", feature = "csr"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	leptos::mount::mount_to_body(|| leptoskit::web::render(view! { <Turntable /> }));
	wasm_bindgen_futures::spawn_local(async {
		let scene = Scene::new(SCENE);
		build(&scene);
		if let Err(error) = scene.flush().await {
			leptos::logging::error!("the turntable could not start: {error}");
		}
	});
}
