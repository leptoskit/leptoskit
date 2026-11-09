//! The binary form of ops and events: round trips, rejection of malformed
//! batches, and sanitizing. None of it needs a GPU.

use leptoskit_3d::*;

fn every_op() -> Vec<Op> {
	vec![
		Op::LoadAsset {
			asset: AssetId(1),
			path: "models/reaper.glb".into(),
		},
		Op::UnloadAsset { asset: AssetId(2) },
		Op::Spawn {
			node: NodeId(3),
			asset: Some(AssetId(1)),
			parent: None,
		},
		Op::Spawn {
			node: NodeId(4),
			asset: None,
			parent: Some(NodeId(3)),
		},
		Op::Remove { node: NodeId(4) },
		Op::SetTransform {
			node: NodeId(3),
			transform: Transform::from_translation(1.0, 2.0, 3.0)
				.with_yaw(0.5)
				.with_scale(2.0),
		},
		Op::SetVisible {
			node: NodeId(3),
			visible: false,
		},
		Op::SetPartVisible {
			node: NodeId(3),
			part: "head_1".into(),
			visible: true,
		},
		Op::Play {
			node: NodeId(3),
			play: Play::new("idle").speed(1.5).once().fade(0.25),
		},
		Op::Stop {
			node: NodeId(3),
			fade: 0.5,
		},
		Op::SetMorphWeight {
			node: NodeId(3),
			part: Some("robe_0".into()),
			target: "billow".into(),
			weight: 0.75,
		},
		Op::SetMorphWeight {
			node: NodeId(3),
			part: None,
			target: "joy".into(),
			weight: 1.0,
		},
		Op::SetBlend {
			node: NodeId(3),
			part: Some("glow".into()),
			blend: Blend::Additive,
		},
		Op::SetBlend {
			node: NodeId(3),
			part: None,
			blend: Blend::Normal,
		},
		Op::SetPartOffset {
			node: NodeId(3),
			part: "head".into(),
			offset: Transform::from_translation(0.0, 0.1, 0.0).with_yaw(0.3),
			seconds: 0.033,
		},
		Op::SetMaterial {
			node: NodeId(3),
			part: Some("spirit_body".into()),
			material: MaterialOverride {
				color: Some([0.2, 0.4, 0.6, 1.0]),
				emissive: Some([0.0, 1.0, 0.5]),
				emissive_strength: Some(4.0),
				opacity: Some(0.5),
				rim: Some(Rim {
					color: [0.5, 0.8, 1.0],
					strength: 2.0,
					power: 3.0,
				}),
			},
		},
		Op::SetMaterial {
			node: NodeId(3),
			part: None,
			material: MaterialOverride::default(),
		},
		Op::SetShadow {
			node: NodeId(3),
			cast: false,
		},
		Op::Attach {
			node: NodeId(5),
			target: NodeId(3),
			bone: "socket_weapon".into(),
		},
		Op::SetCamera {
			transform: Transform::from_translation(0.0, 1.5, 4.0),
			fov_y: 0.8,
			near: 0.05,
			far: 100.0,
		},
		Op::SetCameraMatrices {
			view: std::array::from_fn(|index| index as f32),
			projection: std::array::from_fn(|index| -(index as f32)),
		},
		Op::SetLight(Light::default()),
		Op::SetBackdrop(Backdrop::Transparent),
		Op::SetBackdrop(Backdrop::Color([0.1, 0.2, 0.3, 1.0])),
		Op::SetBackdrop(Backdrop::Camera),
		Op::SetExposure(1.25),
		Op::Raycast {
			request: RequestId(9),
			origin: [0.0, 1.0, 5.0],
			direction: [0.0, 0.0, -1.0],
			max_distance: 50.0,
		},
		Op::RaycastScreen {
			request: RequestId(10),
			x: 0.5,
			y: 0.25,
		},
	]
}

fn every_event() -> Vec<Event> {
	vec![
		Event::AssetLoaded {
			asset: AssetId(1),
			clips: vec!["idle".into(), "walk".into()],
		},
		Event::AssetFailed {
			asset: AssetId(2),
			message: "the file is not valid glTF".into(),
		},
		Event::AnimationFinished {
			node: NodeId(3),
			clip: "slash".into(),
		},
		Event::RaycastHits {
			request: RequestId(9),
			hits: vec![Hit {
				node: NodeId(3),
				part: "helm_0".into(),
				distance: 4.5,
				point: [0.0, 1.0, 0.5],
				normal: [0.0, 0.0, 1.0],
			}],
		},
		Event::RaycastHits {
			request: RequestId(10),
			hits: Vec::new(),
		},
	]
}

#[test]
fn every_op_round_trips() {
	let ops = every_op();
	let bytes = encode_ops(&ops);
	assert!(bytes.starts_with(OPS_MAGIC));
	assert_eq!(decode_ops(&bytes).unwrap(), ops);
}

#[test]
fn every_event_round_trips() {
	let events = every_event();
	let bytes = encode_events(&events);
	assert!(bytes.starts_with(EVENTS_MAGIC));
	assert_eq!(decode_events(&bytes).unwrap(), events);
}

#[test]
fn an_empty_batch_round_trips() {
	assert_eq!(decode_ops(&encode_ops(&[])).unwrap(), []);
	assert_eq!(decode_events(&encode_events(&[])).unwrap(), []);
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
	let bytes = encode_ops(&every_op());

	for length in 0..bytes.len() {
		assert!(
			decode_ops(&bytes[..length]).is_err(),
			"a batch cut at {length} decoded"
		);
	}

	let bytes = encode_events(&every_event());

	for length in 0..bytes.len() {
		assert!(decode_events(&bytes[..length]).is_err());
	}
}

#[test]
fn malformed_batches_are_rejected() {
	let mut bytes = encode_ops(&every_op());
	bytes.push(0);
	assert_eq!(decode_ops(&bytes), Err(DecodeError("bytes are left over")));

	let mut bytes = encode_ops(&[]);
	bytes[0] = b'X';
	assert!(decode_ops(&bytes).is_err());

	let mut bytes = encode_ops(&[]);
	bytes[4] = 99;
	assert_eq!(
		decode_ops(&bytes),
		Err(DecodeError("its version is not supported"))
	);

	// A count far larger than the bytes can hold fails before allocating.
	let mut bytes = encode_ops(&[]);
	bytes[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
	assert!(decode_ops(&bytes).is_err());
	let mut bytes = encode_ops(&[]);
	bytes[5..9].copy_from_slice(&1000_u32.to_le_bytes());
	assert!(decode_ops(&bytes).is_err());

	// An unknown op.
	let mut bytes = encode_ops(&[Op::Remove { node: NodeId(1) }]);
	bytes[9] = 200;
	assert_eq!(decode_ops(&bytes), Err(DecodeError("an unknown op")));

	// A flag that is neither 0 nor 1.
	let mut bytes = encode_ops(&[Op::SetVisible {
		node: NodeId(1),
		visible: true,
	}]);
	*bytes.last_mut().unwrap() = 2;
	assert!(decode_ops(&bytes).is_err());

	// A string that is not UTF-8.
	let mut bytes = encode_ops(&[Op::LoadAsset {
		asset: AssetId(1),
		path: "ab".into(),
	}]);
	let last = bytes.len() - 1;
	bytes[last] = 0xff;
	assert_eq!(
		decode_ops(&bytes),
		Err(DecodeError("a string is not UTF-8"))
	);

	// A batch larger than the limit.
	assert!(decode_ops(&vec![0; MAX_BATCH_BYTES + 1]).is_err());
}

#[test]
fn names_longer_than_their_limit_are_rejected_when_decoding() {
	let bytes = encode_ops(&[Op::SetPartVisible {
		node: NodeId(1),
		part: "x".repeat(MAX_NAME_BYTES + 1),
		visible: true,
	}]);
	assert_eq!(
		decode_ops(&bytes),
		Err(DecodeError("a string is longer than its limit"))
	);
	let bytes = encode_ops(&[Op::LoadAsset {
		asset: AssetId(1),
		path: "x".repeat(MAX_PATH_BYTES),
	}]);
	assert!(decode_ops(&bytes).is_ok());
}

#[test]
fn encoding_cuts_strings_at_a_character_boundary() {
	let long = "é".repeat(40_000);
	let bytes = encode_ops(&[Op::LoadAsset {
		asset: AssetId(1),
		path: long,
	}]);
	let length = u16::from_le_bytes([bytes[14], bytes[15]]);
	// 65 535 would split the last two-byte character.
	assert_eq!(length, u16::MAX - 1);
}

/// Random bytes and random mutations of a valid batch never panic, and
/// whatever decodes re-encodes to the same ops.
#[test]
fn random_and_mutated_batches_never_panic() {
	let mut state = 0x9e37_79b9_u32;
	let mut next = move || {
		state ^= state << 13;
		state ^= state >> 17;
		state ^= state << 5;
		state
	};
	let valid = encode_ops(&every_op());

	for _ in 0..20_000 {
		let mut bytes = valid.clone();

		for _ in 0..=(next() % 4) {
			let index = next() as usize % bytes.len();
			bytes[index] = next() as u8;
		}

		if let Ok(ops) = decode_ops(&bytes) {
			let sanitized: Vec<Op> = ops.iter().cloned().map(Op::sanitized).collect();
			assert_eq!(decode_ops(&encode_ops(&ops)).unwrap().len(), ops.len());
			assert_eq!(sanitized.len(), ops.len());
		}

		let length = next() as usize % 64;
		let mut noise: Vec<u8> = (0..length).map(|_| next() as u8).collect();

		if noise.len() >= 5 && next() % 2 == 0 {
			noise[..4].copy_from_slice(OPS_MAGIC);
			noise[4] = VERSION;
		}

		let _ = decode_ops(&noise);
		let _ = decode_events(&noise);
	}
}

fn finite(values: &[f32]) -> bool {
	values.iter().all(|value| value.is_finite())
}

#[test]
fn sanitizing_bounds_part_offsets() {
	for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0e30, -1.0e30] {
		let Op::SetPartOffset {
			offset, seconds, ..
		} = (Op::SetPartOffset {
			node: NodeId(1),
			part: "head".into(),
			offset: Transform {
				translation: [value; 3],
				rotation: [value; 4],
				scale: [value; 3],
			},
			seconds: value,
		})
		.sanitized()
		else {
			unreachable!()
		};
		assert!(finite(&offset.translation) && finite(&offset.rotation) && finite(&offset.scale));
		assert!((0.0..=MAX_FADE).contains(&seconds));
	}
}

#[test]
fn sanitizing_bounds_every_number() {
	let hostile = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0e30, -1.0e30];

	for value in hostile {
		let transform = Transform {
			translation: [value; 3],
			rotation: [value; 4],
			scale: [value; 3],
		}
		.sanitized();
		assert!(finite(&transform.translation));
		assert!(finite(&transform.scale));
		let length: f32 = transform.rotation.iter().map(|value| value * value).sum();
		assert!((length - 1.0).abs() < 1.0e-5, "{transform:?}");
		assert!(
			transform
				.translation
				.iter()
				.all(|value| value.abs() <= MAX_COORDINATE)
		);

		assert!(transform.scale.iter().all(|value| value.abs() <= MAX_SCALE));

		let Op::Play { play, .. } = (Op::Play {
			node: NodeId(1),
			play: Play::new("idle").speed(value).fade(value),
		})
		.sanitized() else {
			unreachable!()
		};
		assert!(play.speed.abs() <= MAX_SPEED && play.speed.is_finite());
		assert!((0.0..=MAX_FADE).contains(&play.fade));

		let Op::SetCamera {
			fov_y, near, far, ..
		} = (Op::SetCamera {
			transform: Transform::IDENTITY,
			fov_y: value,
			near: value,
			far: value,
		})
		.sanitized()
		else {
			unreachable!()
		};
		assert!((FOV_RANGE.0..=FOV_RANGE.1).contains(&fov_y));
		assert!(
			near >= MIN_NEAR && far > near && far <= MAX_FAR,
			"{near} {far}"
		);

		let Op::SetLight(light) = Op::SetLight(Light {
			direction: [value; 3],
			color: [value; 3],
			intensity: value,
			ambient: value,
			shadow: value,
		})
		.sanitized() else {
			unreachable!()
		};
		let length: f32 = light.direction.iter().map(|value| value * value).sum();
		assert!((length - 1.0).abs() < 1.0e-5);
		assert!(light.color.iter().all(|value| (0.0..=1.0).contains(value)));
		assert!((0.0..=MAX_INTENSITY).contains(&light.intensity));
		assert!((0.0..=1.0).contains(&light.shadow));

		let Op::SetMaterial { material, .. } = (Op::SetMaterial {
			node: NodeId(1),
			part: None,
			material: MaterialOverride {
				color: Some([value; 4]),
				emissive: Some([value; 3]),
				emissive_strength: Some(value),
				opacity: Some(value),
				rim: Some(Rim {
					color: [value; 3],
					strength: value,
					power: value,
				}),
			},
		})
		.sanitized() else {
			unreachable!()
		};
		assert!(
			material
				.color
				.unwrap()
				.iter()
				.all(|value| (0.0..=1.0).contains(value))
		);
		assert!((0.0..=1.0).contains(&material.opacity.unwrap()));
		assert!((0.0..=MAX_INTENSITY).contains(&material.emissive_strength.unwrap()));
		assert!((0.0..=MAX_RIM_POWER).contains(&material.rim.unwrap().power));

		let Op::SetMorphWeight { weight, .. } = (Op::SetMorphWeight {
			node: NodeId(1),
			part: None,
			target: "joy".into(),
			weight: value,
		})
		.sanitized() else {
			unreachable!()
		};
		assert!(weight.abs() <= MAX_MORPH_WEIGHT);

		let Op::Raycast {
			direction,
			max_distance,
			origin,
			..
		} = (Op::Raycast {
			request: RequestId(1),
			origin: [value; 3],
			direction: [value; 3],
			max_distance: value,
		})
		.sanitized()
		else {
			unreachable!()
		};
		assert!(finite(&origin) && finite(&direction) && max_distance.is_finite());
		let length: f32 = direction.iter().map(|value| value * value).sum();
		assert!((length - 1.0).abs() < 1.0e-5);

		let Op::SetCameraMatrices { view, .. } = (Op::SetCameraMatrices {
			view: [value; 16],
			projection: [value; 16],
		})
		.sanitized() else {
			unreachable!()
		};
		assert!(finite(&view));

		let Op::SetExposure(exposure) = Op::SetExposure(value).sanitized() else {
			unreachable!()
		};
		assert!((0.0..=MAX_EXPOSURE).contains(&exposure));
	}
}

#[test]
fn sanitizing_keeps_reasonable_values() {
	for op in every_op() {
		let sanitized = op.clone().sanitized();

		match (&op, &sanitized) {
			// Matrices and the camera's planes are kept; the test values are
			// already within bounds.
			(Op::SetLight(_), Op::SetLight(light)) => {
				let length: f32 = light.direction.iter().map(|value| value * value).sum();
				assert!((length - 1.0).abs() < 1.0e-5);
			}
			(Op::SetTransform { .. }, Op::SetTransform { transform, .. }) => {
				assert_eq!(transform.translation, [1.0, 2.0, 3.0]);
				assert_eq!(transform.scale, [2.0; 3]);
			}
			_ => assert_eq!(op, sanitized),
		}
	}
}

#[test]
fn sanitizing_cuts_long_names_at_a_character_boundary() {
	let Op::SetPartVisible { part, .. } = (Op::SetPartVisible {
		node: NodeId(1),
		part: "é".repeat(MAX_NAME_BYTES),
		visible: true,
	})
	.sanitized() else {
		unreachable!()
	};
	assert!(part.len() <= MAX_NAME_BYTES);
	assert!(part.chars().all(|character| character == 'é'));
}

#[test]
fn asset_paths_must_stay_inside_the_assets_folder() {
	for path in [
		"models/reaper.glb",
		"reaper.glb",
		"a/b/c.gltf",
		"textures/skin%20tone.png",
	] {
		assert!(is_safe_asset_path(path), "{path}");
	}
	for path in [
		"",
		"/etc/passwd",
		"../secret.glb",
		"models/../../secret.glb",
		"models/./reaper.glb",
		"models//reaper.glb",
		"C:/Windows/system.ini",
		"models\\reaper.glb",
		"models/reaper.glb\0",
		"models/",
	] {
		assert!(!is_safe_asset_path(path), "{path:?}");
	}
	assert!(!is_safe_asset_path(&"a".repeat(MAX_PATH_BYTES + 1)));
}
