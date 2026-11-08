//! The retained scene, driven by ops alone and checked through events and
//! raycasts. None of it needs a GPU.
#![cfg(feature = "renderer")]

mod support;

use std::sync::Arc;

use leptoskit_3d::*;
use support::fixture;

const RIG: AssetId = AssetId(1);
const FIGURE: NodeId = NodeId(10);

/// A scene with the rig loaded and spawned at the origin.
fn scene() -> World {
	let mut world = World::new();
	world.apply([
		Op::LoadAsset {
			asset: RIG,
			path: "models/rig.glb".into(),
		},
		Op::Spawn {
			node: FIGURE,
			asset: Some(RIG),
			parent: None,
		},
	]);
	assert_eq!(world.take_loads(), [(RIG, "models/rig.glb".to_owned())]);
	let asset = Asset::from_slice(&fixture::rig()).unwrap();
	world.asset_loaded(RIG, Ok(Arc::new(asset)));
	world.advance(0.0);
	world.take_events();
	world
}

/// The hits of a ray along -z towards the rig, through (x, y).
fn hits(world: &mut World, x: f32, y: f32) -> Vec<Hit> {
	world.apply([Op::Raycast {
		request: RequestId(1),
		origin: [x, y, 5.0],
		direction: [0.0, 0.0, -1.0],
		max_distance: 100.0,
	}]);

	match world.take_events().pop() {
		Some(Event::RaycastHits { request, hits }) => {
			assert_eq!(request, RequestId(1));
			hits
		}
		other => panic!("expected raycast hits, got {other:?}"),
	}
}

/// Advances `seconds` in display frames: one step of `advance` is at most a
/// quarter of a second, so a suspended app does not jump.
fn run(world: &mut World, seconds: f32) {
	let mut left = seconds;

	while left > 0.0 {
		let step = left.min(1.0 / 60.0);
		world.advance(step);
		left -= step;
	}
}

fn parts(hits: &[Hit]) -> Vec<&str> {
	hits.iter().map(|hit| hit.part.as_str()).collect()
}

#[test]
fn the_fixture_loads_with_its_names_clips_and_targets() {
	let asset = Asset::from_slice(&fixture::rig()).unwrap();
	assert_eq!(
		asset.node_names().collect::<Vec<_>>(),
		["rig", "hip", "arm", "body", "head", "socket"]
	);
	assert_eq!(
		asset.clip_names().collect::<Vec<_>>(),
		["raise", "bob", "swell", "nod"]
	);
	assert_eq!(asset.morph_target_names(), ["grow", "shrink"]);
	assert_eq!(asset.skin_count(), 1);
	assert_eq!(asset.mesh_count(), (2, 2));
	assert_eq!(asset.vertex_count(), 48);
	assert_eq!(asset.triangle_count(), 24);

	assert_eq!(asset.clip_duration("raise"), Some(1.0));
}

#[test]
fn loading_reports_the_clips_and_instances_appear() {
	let mut world = World::new();
	world.apply([
		Op::Spawn {
			node: FIGURE,
			asset: Some(RIG),
			parent: None,
		},
		Op::LoadAsset {
			asset: RIG,
			path: "models/rig.glb".into(),
		},
		// Played before the asset loads: it starts when it does.
		Op::Play {
			node: FIGURE,
			play: Play::new("raise").once(),
		},
	]);
	world.advance(0.0);
	assert!(
		hits(&mut world, 0.0, 1.0).is_empty(),
		"nothing draws before loading"
	);
	let loads = world.take_loads();
	assert_eq!(loads.len(), 1);
	world.asset_loaded(
		RIG,
		Ok(Arc::new(Asset::from_slice(&fixture::rig()).unwrap())),
	);
	assert_eq!(
		world.take_events(),
		[Event::AssetLoaded {
			asset: RIG,
			clips: vec!["raise".into(), "bob".into(), "swell".into(), "nod".into()],
		}]
	);

	run(&mut world, 2.0);
	assert!(world.take_events().contains(&Event::AnimationFinished {
		node: FIGURE,
		clip: "raise".into(),
	}));
	// A second answer for the same asset is ignored.
	world.asset_loaded(RIG, Err("late".into()));
	assert_eq!(world.take_events(), [] as [Event; 0]);
}

#[test]
fn failures_and_unsafe_paths_are_reported_without_loading() {
	let mut world = World::new();
	world.apply([
		Op::LoadAsset {
			asset: AssetId(1),
			path: "../outside.glb".into(),
		},
		Op::LoadAsset {
			asset: AssetId(2),
			path: "models/broken.glb".into(),
		},
	]);
	assert_eq!(
		world.take_loads(),
		[(AssetId(2), "models/broken.glb".to_owned())]
	);
	world.asset_loaded(AssetId(2), Err("the file is not valid glTF".into()));
	let events = world.take_events();
	assert!(matches!(
		&events[0],
		Event::AssetFailed {
			asset: AssetId(1),
			..
		}
	));
	assert_eq!(
		events[1],
		Event::AssetFailed {
			asset: AssetId(2),
			message: "the file is not valid glTF".into(),
		}
	);
}

#[test]
fn raycasts_meet_parts_nearest_first() {
	let mut world = scene();
	let hits = hits(&mut world, 0.0, 1.0);
	assert_eq!(parts(&hits), ["body"]);
	assert!((hits[0].distance - 4.8).abs() < 1.0e-4, "{:?}", hits[0]);
	assert_eq!(hits[0].normal, [0.0, 0.0, 1.0]);
	assert_eq!(hits[0].node, FIGURE);
	assert_eq!(parts(&self::hits(&mut world, 0.0, 1.8)), ["head"]);
	assert_eq!(self::hits(&mut world, 2.0, 1.0), [] as [Hit; 0]);
}

#[test]
fn hidden_parts_and_nodes_are_not_hit() {
	let mut world = scene();
	world.apply([Op::SetPartVisible {
		node: FIGURE,
		part: "body".into(),
		visible: false,
	}]);
	assert_eq!(hits(&mut world, 0.0, 1.0), [] as [Hit; 0]);
	world.apply([
		Op::SetPartVisible {
			node: FIGURE,
			part: "body".into(),
			visible: true,
		},
		Op::SetVisible {
			node: FIGURE,
			visible: false,
		},
	]);
	assert_eq!(hits(&mut world, 0.0, 1.0), [] as [Hit; 0]);
	world.apply([Op::SetVisible {
		node: FIGURE,
		visible: true,
	}]);
	assert_eq!(parts(&hits(&mut world, 0.0, 1.0)), ["body"]);
	// Parts that do not exist are ignored.
	world.apply([Op::SetPartVisible {
		node: FIGURE,
		part: "tail".into(),
		visible: false,
	}]);

	assert_eq!(parts(&hits(&mut world, 0.0, 1.0)), ["body"]);
}

#[test]
fn transforms_move_instances() {
	let mut world = scene();
	world.apply([Op::SetTransform {
		node: FIGURE,
		transform: Transform::from_translation(2.0, 0.0, 0.0),
	}]);
	assert_eq!(hits(&mut world, 0.0, 1.0), [] as [Hit; 0]);
	assert_eq!(parts(&hits(&mut world, 2.0, 1.0)), ["body"]);
	world.apply([Op::SetTransform {
		node: FIGURE,
		transform: Transform::from_translation(0.0, 0.0, 0.0).with_scale(2.0),
	}]);
	// Twice the size: the head now reaches 4 m.
	assert_eq!(parts(&hits(&mut world, 0.0, 3.6)), ["head"]);
}

#[test]
fn skinning_follows_the_joints_and_once_clips_finish() {
	let mut world = scene();
	// The arm turns the body's top 90° about z around (0, 0.8): the top face
	// ends up upright at x = -0.8, between y = 0.5 and 1.1, so the bent box
	// covers (-0.65, 0.8), which is outside it at rest.
	assert_eq!(hits(&mut world, -0.65, 0.8), [] as [Hit; 0]);
	world.apply([Op::Play {
		node: FIGURE,
		play: Play::new("raise").once(),
	}]);
	assert!(world.needs_frame());
	run(&mut world, 0.5);
	assert_eq!(world.take_events(), [] as [Event; 0]);
	run(&mut world, 0.6);
	assert_eq!(
		world.take_events(),
		[Event::AnimationFinished {
			node: FIGURE,
			clip: "raise".into(),
		}]
	);

	assert_eq!(parts(&hits(&mut world, -0.65, 0.8)), ["body"]);
	// It holds its last pose and asks for no more frames.
	run(&mut world, 0.1);
	assert!(!world.needs_frame());
	assert_eq!(parts(&hits(&mut world, -0.65, 0.8)), ["body"]);
}

#[test]
fn step_keys_hold_and_loops_keep_asking_for_frames() {
	let mut world = scene();
	// Step interpolation: the rig jumps up 0.5 m at the clip's last key.
	world.apply([Op::Play {
		node: FIGURE,
		play: Play::new("bob").once(),
	}]);
	run(&mut world, 0.9);
	assert_eq!(parts(&hits(&mut world, 0.0, 0.1)), ["body"]);
	run(&mut world, 0.2);
	assert!(
		hits(&mut world, 0.0, 0.1).is_empty(),
		"the rig jumped to the last key"
	);
	world.apply([Op::Play {
		node: FIGURE,
		play: Play::new("bob"),
	}]);
	run(&mut world, 3.0);

	assert!(world.needs_frame(), "a loop never settles");
	assert_eq!(parts(&hits(&mut world, 0.0, 0.1)), ["body"]);
	world.apply([Op::Stop {
		node: FIGURE,
		fade: 0.0,
	}]);
	world.advance(0.0);
	assert_eq!(parts(&hits(&mut world, 0.0, 0.1)), ["body"]);
	assert!(!world.needs_frame());
}

#[test]
fn crossfades_blend_and_fading_stops_settle() {
	let mut world = scene();
	world.apply([Op::Play {
		node: FIGURE,
		play: Play::new("raise").once(),
	}]);
	run(&mut world, 1.0);
	world.take_events();
	// Fading into a clip that does not move the arm brings it back over 1 s.
	world.apply([Op::Play {
		node: FIGURE,
		play: Play::new("swell").fade(1.0),
	}]);
	run(&mut world, 0.5);
	assert!(world.needs_frame());
	run(&mut world, 0.6);

	world.apply([Op::Stop {
		node: FIGURE,
		fade: 0.5,
	}]);
	run(&mut world, 0.3);
	assert!(world.needs_frame());
	run(&mut world, 0.3);
	assert!(!world.needs_frame(), "the stop has faded out");
	assert!(hits(&mut world, -0.65, 0.8).is_empty(), "back at rest");
}

#[test]
fn morph_weights_change_the_shape() {
	let mut world = scene();
	// The head is 0.4 m wide; at x = 0.25 a ray misses it.
	assert_eq!(hits(&mut world, 0.25, 1.8), [] as [Hit; 0]);
	world.apply([Op::SetMorphWeight {
		node: FIGURE,
		part: None,
		target: "grow".into(),
		weight: 1.0,
	}]);
	world.advance(0.0);
	assert_eq!(parts(&hits(&mut world, 0.25, 1.8)), ["head"]);
	// The manual weight replaces what a clip sets for that target.
	world.apply([Op::Play {
		node: FIGURE,
		play: Play::new("swell"),
	}]);
	world.advance(0.0);
	assert_eq!(parts(&hits(&mut world, 0.25, 1.8)), ["head"]);

	world.apply([Op::SetMorphWeight {
		node: FIGURE,
		part: Some("head".into()),
		target: "grow".into(),
		weight: 0.0,
	}]);
	world.advance(0.0);
	assert_eq!(hits(&mut world, 0.25, 1.8), [] as [Hit; 0]);
	// Targets on other parts, and unknown targets, change nothing.
	world.apply([
		Op::SetMorphWeight {
			node: FIGURE,
			part: Some("body".into()),
			target: "grow".into(),
			weight: 1.0,
		},
		Op::SetMorphWeight {
			node: FIGURE,
			part: None,
			target: "missing".into(),
			weight: 1.0,
		},
	]);
	world.advance(0.0);
	assert_eq!(hits(&mut world, 0.25, 1.8), [] as [Hit; 0]);
}

#[test]
fn attached_nodes_follow_their_part() {
	let mut world = scene();
	let cube = AssetId(2);
	let sword = NodeId(20);
	world.apply([
		Op::LoadAsset {
			asset: cube,
			path: "models/cube.glb".into(),
		},
		Op::Spawn {
			node: sword,
			asset: Some(cube),
			parent: None,
		},
		Op::SetTransform {
			node: sword,
			transform: Transform::IDENTITY.with_scale(0.2),
		},
		Op::Attach {
			node: sword,
			target: FIGURE,
			bone: "socket".into(),
		},
	]);
	world.asset_loaded(
		cube,
		Ok(Arc::new(
			Asset::from_slice(&fixture::unlit_cube([1.0; 4], false)).unwrap(),
		)),
	);
	world.advance(0.0);
	world.take_events();
	// The socket is at (0.5, 1); the cube is 0.2 m across around it.
	let hits = hits(&mut world, 0.55, 1.0);

	assert_eq!(hits.len(), 1);
	assert_eq!(hits[0].node, sword);
	assert_eq!(hits[0].part, "cube");
	world.apply([Op::SetTransform {
		node: FIGURE,
		transform: Transform::from_translation(3.0, 0.0, 0.0),
	}]);
	assert_eq!(self::hits(&mut world, 0.55, 1.0), [] as [Hit; 0]);
	assert_eq!(self::hits(&mut world, 3.55, 1.0)[0].node, sword);
	// Hiding the part hides what is attached to it.
	world.apply([Op::SetPartVisible {
		node: FIGURE,
		part: "socket".into(),
		visible: false,
	}]);
	assert_eq!(self::hits(&mut world, 3.55, 1.0), [] as [Hit; 0]);
}

#[test]
fn attaching_refuses_cycles_and_removing_takes_the_subtree() {
	let mut world = scene();
	world.apply([
		Op::Spawn {
			node: NodeId(1),
			asset: None,
			parent: None,
		},
		Op::Spawn {
			node: NodeId(2),
			asset: None,
			parent: Some(NodeId(1)),
		},
		Op::Spawn {
			node: NodeId(3),
			asset: None,
			parent: Some(NodeId(2)),
		},
		// Would put 1 under its own grandchild.
		Op::Attach {
			node: NodeId(1),
			target: NodeId(3),
			bone: String::new(),
		},
		Op::Attach {
			node: NodeId(1),
			target: NodeId(1),
			bone: String::new(),
		},
		// A duplicate id is ignored.
		Op::Spawn {
			node: NodeId(2),
			asset: None,
			parent: None,
		},
	]);
	world.advance(0.0);
	assert_eq!(world.node_count(), 4);
	world.apply([Op::Remove { node: NodeId(1) }]);
	assert_eq!(world.node_count(), 1);
	world.apply([Op::Attach {
		node: FIGURE,
		target: NodeId(3),
		bone: "socket".into(),
	}]);
	world.advance(0.0);

	assert_eq!(world.node_count(), 1);
}

#[test]
fn scenes_hold_a_bounded_number_of_nodes_and_events() {
	let mut world = World::new();
	let spawns: Vec<Op> = (0..MAX_NODES as u32 + 10)
		.map(|id| {
			Op::Spawn {
				node: NodeId(id),
				asset: None,
				parent: None,
			}
		})
		.collect();
	world.apply(spawns);
	assert_eq!(world.node_count(), MAX_NODES);

	let raycasts: Vec<Op> = (0..MAX_PENDING_EVENTS as u32 + 5)
		.map(|request| {
			Op::Raycast {
				request: RequestId(request),
				origin: [0.0; 3],
				direction: [0.0, 0.0, -1.0],
				max_distance: 1.0,
			}
		})
		.collect();
	world.apply(raycasts);
	let events = world.take_events();
	assert_eq!(events.len(), MAX_PENDING_EVENTS);
	assert_eq!(world.dropped_events(), 5);
	assert!(
		matches!(events.last(), Some(Event::RaycastHits { request, .. }) if request.0 as usize == MAX_PENDING_EVENTS + 4)
	);
}

#[test]
fn screen_raycasts_go_through_the_camera() {
	let mut world = scene();
	world.apply([Op::SetCamera {
		transform: Transform::from_translation(0.0, 1.0, 5.0),
		fov_y: 0.8,
		near: 0.1,
		far: 100.0,
	}]);
	world.set_aspect(1.0);
	world.apply([
		Op::RaycastScreen {
			request: RequestId(7),
			x: 0.5,
			y: 0.5,
		},
		Op::RaycastScreen {
			request: RequestId(8),
			x: 0.0,
			y: 0.0,
		},
	]);
	let events = world.take_events();
	let Event::RaycastHits { request, hits } = &events[0] else {
		panic!("{events:?}")
	};
	assert_eq!(*request, RequestId(7));

	assert_eq!(parts(hits), ["body"]);
	// The ray starts on the near plane, 0.1 m in front of the camera.
	assert!((hits[0].distance - 4.7).abs() < 1.0e-3, "{:?}", hits[0]);
	assert_eq!(
		events[1],
		Event::RaycastHits {
			request: RequestId(8),
			hits: Vec::new(),
		}
	);
}

#[test]
fn operations_on_missing_nodes_are_ignored() {
	let mut world = World::new();
	world.apply([
		Op::SetTransform {
			node: NodeId(5),
			transform: Transform::IDENTITY,
		},
		Op::Play {
			node: NodeId(5),
			play: Play::new("idle"),
		},
		Op::SetMaterial {
			node: NodeId(5),
			part: None,
			material: MaterialOverride::default(),
		},
		Op::Remove { node: NodeId(5) },
		Op::UnloadAsset { asset: AssetId(9) },
	]);
	world.advance(0.0);
	assert_eq!(world.node_count(), 0);
	assert_eq!(world.take_events(), [] as [Event; 0]);
}
