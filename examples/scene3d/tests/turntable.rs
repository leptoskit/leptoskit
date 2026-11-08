//! The turntable's screen validates, its model loads with the clip it
//! plays, and a frame of it draws (on machines with a GPU adapter).
#![recursion_limit = "256"]

use std::sync::Arc;

use leptoskit::scene3d::client::Scene;
use leptoskit_3d::Asset;
use leptoskit_3d::Renderer;
use scene3d_example::MODEL;
use scene3d_example::SCENE;

fn model() -> Arc<Asset> {
	let bytes = std::fs::read(format!("{}/assets/{MODEL}", env!("CARGO_MANIFEST_DIR"))).unwrap();
	Arc::new(Asset::from_slice(&bytes).unwrap())
}

#[test]
fn the_screen_is_valid() {
	scene3d_example::Turntable().validate().unwrap();
}

#[test]
fn the_model_has_a_skin_and_the_clip_the_scene_plays() {
	let model = model();
	assert_eq!(model.skin_count(), 1);
	assert!(model.clip_names().any(|clip| clip == "spin"));
	assert_eq!(model.clip_duration("spin"), Some(4.0));
}

#[test]
fn a_frame_of_the_turntable_draws_the_character() {
	let gpu = match leptoskit_3d::Gpu::headless() {
		Ok(gpu) => gpu,

		Err(error) if error.is_unavailable() => return eprintln!("skipping: {error}"),
		Err(error) => panic!("{error}"),
	};

	let scene = Scene::new(SCENE);
	scene3d_example::build(&scene);
	let mut renderer = Renderer::new(&gpu);
	let world = renderer.world_mut();
	world.apply(scene.take_ops());

	for (asset, path) in world.take_loads() {
		assert_eq!(path, MODEL);

		world.asset_loaded(asset, Ok(model()));
	}

	for _ in 0..30 {
		renderer.advance(1.0 / 60.0);
	}

	let (width, height) = (96, 128);
	let pixels = renderer.render_to_rgba(width, height).unwrap();
	let centre = ((height / 2 * width + width / 2) * 4) as usize;
	assert_eq!(pixels[centre + 3], 255, "the character covers the middle");
	assert!(renderer.world().needs_frame(), "the spin keeps it moving");
}
