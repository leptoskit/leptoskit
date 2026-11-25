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
	.into_iter()
	.chain(style_ops())
	.chain(print_ops())
	.chain(motion_ops())
	.collect()
}

/// Relief-print styles, overrides, and camera-space lights.
fn print_ops() -> Vec<Op> {
	vec![
		Op::SetStyle(Style::print()),
		Op::SetStyle(Style {
			shading: Shading::Print(Print {
				t_lit: 0.4,
				t_deep: -0.2,
				t_cross: -0.6,
				rim: 0.25,
				paper: [0.9, 0.85, 0.7],
				knockout_width: 3.0,
				hatch_per_m: 42.0,
				hatch_gain: 0.6,
				hatch_max: 0.7,
				cross_duty: 0.35,
				fade_px: [2.0, 5.0],
				side_heavy: 1.4,
				side_drop: 0.7,
				side_from: -0.4,
				side_to: 0.8,
				wobble_frequency: 6.0,
			}),
			ink: None,
		}),
		Op::SetPrint {
			node: NodeId(3),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some([0.9, 0.8, 0.7]),
				shade: Some([0.6, 0.5, 0.4]),
				deep: Some([0.3, 0.2, 0.1]),
				spot: Some([0.95, 0.3, 0.1]),
				mode: Some(PrintMode::Banded),
				hatch_min: Some(0.18),
				hatch_per_m: Some(36.0),
				cross_hatch: Some(true),
				specular: Some(0.92),
				rim: Some(true),
				ink_weight: Some(1.25),
				knockout: Some(true),
				boil_step: Some(2),
			},
		},
		Op::SetPrint {
			node: NodeId(3),
			target: PrintTarget::Part("head".into()),
			print: PrintOverride {
				lit: Some([1.0, 0.9, 0.8]),
				mode: Some(PrintMode::Flat),
				knockout: Some(false),
				..PrintOverride::default()
			},
		},
		Op::SetPrint {
			node: NodeId(3),
			target: PrintTarget::Material("void".into()),
			print: PrintOverride::default(),
		},
		Op::SetLight(Light {
			camera_space: true,
			..Light::default()
		}),
	]
}

/// Eased transforms, and clips that ease their crossfade or start at a
/// phase.
fn motion_ops() -> Vec<Op> {
	let ease = |easing| {
		Op::EaseTransform {
			node: NodeId(3),
			transform: Transform::from_translation(0.5, 0.0, -1.0).with_yaw(1.25),
			seconds: 0.4,
			easing,
		}
	};

	vec![
		ease(Easing::Linear),
		ease(Easing::Smooth),
		ease(Easing::In),
		ease(Easing::Out),
		Op::Play {
			node: NodeId(3),
			play: Play::new("walk").fade(0.3).fade_easing(Easing::Smooth),
		},
		Op::Play {
			node: NodeId(3),
			play: Play::new("walk").phase(0.75),
		},
		Op::Play {
			node: NodeId(3),
			play: Play::new("walk")
				.speed(-1.0)
				.once()
				.fade(0.2)
				.fade_easing(Easing::Out)
				.phase(0.0),
		},
	]
}

/// The scene's style and its parts' lines.
fn style_ops() -> Vec<Op> {
	vec![
		Op::SetStyle(Style::default()),
		Op::SetStyle(Style::hand_drawn()),
		Op::SetStyle(Style {
			shading: Shading::Toon(Toon {
				bands: 2,
				softness: 0.25,
				shade: [0.3, 0.2, 0.5],
				specular: 2.0,
				hatch: 0.6,
			}),
			ink: Some(Ink {
				color: [0.9, 0.85, 0.7],
				width: 3.5,
				facing: 0.8,
				shade_weight: 1.5,
				wobble: 0.4,
				boil: 8.0,
				seed: 0xDEAD_BEEF,
			}),
		}),
		Op::SetInk {
			node: NodeId(3),
			part: Some("robe".into()),
			ink: PartInk {
				weight: Some(0.0),
				color: Some([0.8, 0.75, 0.6]),
			},
		},
		Op::SetInk {
			node: NodeId(3),
			part: None,
			ink: PartInk::default(),
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

	// An unknown shading.
	let mut bytes = encode_ops(&[Op::SetStyle(Style::default())]);
	bytes[10] = 2;
	assert_eq!(decode_ops(&bytes), Err(DecodeError("an unknown shading")));

	// An unknown print target.
	let mut bytes = encode_ops(&[Op::SetPrint {
		node: NodeId(1),
		target: PrintTarget::All,
		print: PrintOverride::default(),
	}]);
	bytes[14] = 3;
	assert_eq!(
		decode_ops(&bytes),
		Err(DecodeError("an unknown print target"))
	);

	// An unknown print mode.
	let mut bytes = encode_ops(&[Op::SetPrint {
		node: NodeId(1),
		target: PrintTarget::All,
		print: PrintOverride {
			mode: Some(PrintMode::Flat),
			..PrintOverride::default()
		},
	}]);
	// Header(9) + tag(1) + node(4) + target All(1) + 4 absent colour options(4)
	// + mode Some(1)
	bytes[20] = 2;
	assert_eq!(
		decode_ops(&bytes),
		Err(DecodeError("an unknown print mode"))
	);

	// An unknown easing, of a transform and of a crossfade.
	let mut bytes = encode_ops(&[Op::EaseTransform {
		node: NodeId(1),
		transform: Transform::IDENTITY,
		seconds: 1.0,
		easing: Easing::Out,
	}]);
	*bytes.last_mut().unwrap() = 4;
	assert_eq!(decode_ops(&bytes), Err(DecodeError("an unknown easing")));
	let mut bytes = encode_ops(&[Op::Play {
		node: NodeId(1),
		play: Play::new("idle").fade_easing(Easing::Smooth),
	}]);
	// The easing comes before the absent phase's flag.
	let easing = bytes.len() - 2;
	bytes[easing] = 9;
	assert_eq!(decode_ops(&bytes), Err(DecodeError("an unknown easing")));

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
	for target in [
		PrintTarget::Part("x".repeat(MAX_NAME_BYTES + 1)),
		PrintTarget::Material("x".repeat(MAX_NAME_BYTES + 1)),
	] {
		let bytes = encode_ops(&[Op::SetPrint {
			node: NodeId(1),
			target,
			print: PrintOverride::default(),
		}]);
		assert_eq!(
			decode_ops(&bytes),
			Err(DecodeError("a string is longer than its limit"))
		);
	}
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
fn sanitizing_bounds_eases_and_phases() {
	for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0e30, -1.0e30] {
		let Op::EaseTransform {
			transform,
			seconds,
			easing,
			..
		} = (Op::EaseTransform {
			node: NodeId(1),
			transform: Transform {
				translation: [value; 3],
				rotation: [value; 4],
				scale: [value; 3],
			},
			seconds: value,
			easing: Easing::In,
		})
		.sanitized()
		else {
			unreachable!()
		};
		assert!(
			finite(&transform.translation)
				&& finite(&transform.rotation)
				&& finite(&transform.scale)
		);
		assert!((0.0..=MAX_FADE).contains(&seconds));
		assert_eq!(easing, Easing::In);

		let Op::Play { play, .. } = (Op::Play {
			node: NodeId(1),
			play: Play::new("idle").phase(value),
		})
		.sanitized() else {
			unreachable!()
		};
		assert!(play.phase.is_some_and(|phase| (0.0..=1.0).contains(&phase)));
	}

	// A phase past the clip's end is its end, and no phase stays none.
	let phase = |play: Play| {
		let Op::Play { play, .. } = (Op::Play {
			node: NodeId(1),
			play,
		})
		.sanitized() else {
			unreachable!()
		};
		play.phase
	};
	assert_eq!(phase(Play::new("idle").phase(1.5)), Some(1.0));
	assert_eq!(phase(Play::new("idle").phase(-0.5)), Some(0.0));
	assert_eq!(phase(Play::new("idle")), None);
}

#[test]
fn easings_run_from_nothing_to_all_of_the_way() {
	let easings = [Easing::Linear, Easing::Smooth, Easing::In, Easing::Out];

	for easing in easings {
		assert_eq!(easing.apply(0.0), 0.0, "{easing:?}");
		assert_eq!(easing.apply(1.0), 1.0, "{easing:?}");
		// Clamped outside the way.
		assert_eq!(easing.apply(-1.0), 0.0, "{easing:?}");
		assert_eq!(easing.apply(2.0), 1.0, "{easing:?}");
		// Never turning back.
		let steps: Vec<f32> = (0..=64)
			.map(|step| easing.apply(step as f32 / 64.0))
			.collect();
		assert!(
			steps.windows(2).all(|pair| pair[0] <= pair[1]),
			"{easing:?}"
		);
	}

	assert_eq!(Easing::default(), Easing::Linear);
	assert_eq!(Easing::Linear.apply(0.25), 0.25);
	// Smoothstep: 3t² − 2t³.
	assert_eq!(Easing::Smooth.apply(0.25), 5.0 / 32.0);
	assert_eq!(Easing::Smooth.apply(0.5), 0.5);
	assert_eq!(Easing::In.apply(0.5), 0.125);
	assert_eq!(Easing::Out.apply(0.5), 0.875);
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
			camera_space: false,
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
fn sanitizing_bounds_styles_and_lines() {
	for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0e30, -1.0e30] {
		let Op::SetStyle(style) = Op::SetStyle(Style {
			shading: Shading::Toon(Toon {
				bands: if value > 0.0 { u8::MAX } else { 0 },
				softness: value,
				shade: [value; 3],
				specular: value,
				hatch: value,
			}),
			ink: Some(Ink {
				color: [value; 3],
				width: value,
				facing: value,
				shade_weight: value,
				wobble: value,
				boil: value,
				seed: 7,
			}),
		})
		.sanitized() else {
			unreachable!()
		};
		let Shading::Toon(toon) = style.shading else {
			unreachable!()
		};
		let ink = style.ink.unwrap();
		let unit = |value: f32| (0.0..=1.0).contains(&value);
		assert!(
			(TOON_BANDS.0..=TOON_BANDS.1).contains(&toon.bands),
			"{toon:?}"
		);
		assert!(unit(toon.softness) && unit(toon.hatch), "{toon:?}");
		assert!(toon.shade.into_iter().all(unit), "{toon:?}");
		assert!((0.0..=MAX_SPECULAR).contains(&toon.specular), "{toon:?}");
		assert!(ink.color.into_iter().all(unit), "{ink:?}");
		assert!((0.0..=MAX_INK_WIDTH).contains(&ink.width), "{ink:?}");
		assert!(unit(ink.facing) && unit(ink.wobble), "{ink:?}");
		assert!(
			(0.0..=MAX_SHADE_WEIGHT).contains(&ink.shade_weight),
			"{ink:?}"
		);
		assert!((0.0..=MAX_BOIL).contains(&ink.boil), "{ink:?}");
		assert_eq!(ink.seed, 7);

		let Op::SetInk { ink, part, .. } = (Op::SetInk {
			node: NodeId(1),
			part: Some("é".repeat(MAX_NAME_BYTES)),
			ink: PartInk {
				weight: Some(value),
				color: Some([value; 3]),
			},
		})
		.sanitized() else {
			unreachable!()
		};
		assert!(part.unwrap().len() <= MAX_NAME_BYTES);
		assert!(
			(0.0..=MAX_INK_WEIGHT).contains(&ink.weight.unwrap()),
			"{ink:?}"
		);
		assert!(ink.color.unwrap().into_iter().all(unit), "{ink:?}");
	}
}

#[test]
fn styles_and_lines_are_new_ops_that_leave_old_batches_alone() {
	// Batches without the new ops encode exactly as before: only the new
	// tags carry them.
	let bytes = encode_ops(&[
		Op::SetStyle(Style::default()),
		Op::SetInk {
			node: NodeId(9),
			part: None,
			ink: PartInk::default(),
		},
	]);
	assert_eq!(&bytes[..9], b"LK3D\x01\x02\x00\x00\x00");
	// SetStyle: tag 23, PBR, no ink.
	assert_eq!(&bytes[9..12], &[23, 0, 0]);
	// SetInk: tag 24, node 9, no part, no weight, no colour.
	assert_eq!(&bytes[12..], &[24, 9, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn eases_and_phases_are_new_ops_that_leave_old_plays_alone() {
	let play = Play::new("idle").speed(1.5).once().fade(0.25);
	// Tag 8, node 3, the clip, speed 1.5, once, a fade of 0.25: a play that
	// neither eases nor starts at a phase is written as it always was.
	let plain: Vec<u8> = [8, 3, 0, 0, 0, 4, 0]
		.into_iter()
		.chain(*b"idle")
		.chain(1.5_f32.to_le_bytes())
		.chain([0])
		.chain(0.25_f32.to_le_bytes())
		.collect();
	let encoded = |play: Play| {
		encode_ops(&[Op::Play {
			node: NodeId(3),
			play,
		}])[9..]
			.to_vec()
	};
	assert_eq!(encoded(play.clone()), plain);
	assert_eq!(
		encoded(play.clone().fade_easing(Easing::Linear)),
		plain,
		"a linear fade is the default, not a change"
	);

	// One that does takes tag 26, with its easing and phase after.
	let mut eased = plain.clone();
	eased[0] = 26;
	eased.extend([1, 0]);
	assert_eq!(encoded(play.clone().fade_easing(Easing::Smooth)), eased);
	let mut phased = plain;
	phased[0] = 26;
	phased.extend([0, 1]);
	phased.extend(0.5_f32.to_le_bytes());
	assert_eq!(encoded(play.phase(0.5)), phased);

	// EaseTransform: tag 25, node 3, the transform, the seconds, the easing.
	let bytes = encode_ops(&[Op::EaseTransform {
		node: NodeId(3),
		transform: Transform::IDENTITY,
		seconds: 0.5,
		easing: Easing::Smooth,
	}]);
	assert_eq!(bytes[9], 25);
	assert_eq!(bytes.len(), 9 + 1 + 4 + 40 + 4 + 1);
	assert_eq!(bytes.last(), Some(&1));
}

fn assert_print_sanitized(value: f32) {
	let unit = |v: f32| (0.0..=1.0).contains(&v);
	let Op::SetStyle(style) = Op::SetStyle(Style {
		shading: Shading::Print(Print {
			t_lit: value,
			t_deep: -value,
			t_cross: value,
			rim: value,
			paper: [value; 3],
			knockout_width: value,
			hatch_per_m: value,
			hatch_gain: value,
			hatch_max: value,
			cross_duty: value,
			fade_px: [value, -value],
			side_heavy: value,
			side_drop: value,
			side_from: value,
			side_to: -value,
			wobble_frequency: value,
		}),
		ink: Some(Ink::default()),
	})
	.sanitized() else {
		unreachable!()
	};
	let Shading::Print(print) = style.shading else {
		unreachable!()
	};
	assert!((-1.0..=1.0).contains(&print.t_lit), "{print:?}");
	assert!((-1.0..=print.t_lit).contains(&print.t_deep), "{print:?}");
	assert!((-2.0..=print.t_deep).contains(&print.t_cross), "{print:?}");
	assert!(unit(print.rim), "{print:?}");
	assert!(print.paper.into_iter().all(unit), "{print:?}");
	assert!(
		(0.0..=MAX_INK_WIDTH).contains(&print.knockout_width),
		"{print:?}"
	);
	assert!(
		(0.0..=MAX_HATCH_PER_M).contains(&print.hatch_per_m),
		"{print:?}"
	);
	assert!(
		(0.0..=MAX_HATCH_GAIN).contains(&print.hatch_gain),
		"{print:?}"
	);
	assert!(unit(print.hatch_max) && unit(print.cross_duty), "{print:?}");
	assert!(
		(MIN_FADE_PX..=MAX_FADE_PX).contains(&print.fade_px[0])
			&& (print.fade_px[0]..=MAX_FADE_PX).contains(&print.fade_px[1]),
		"{print:?}"
	);
	assert!(
		(0.0..=MAX_SIDE_WEIGHT).contains(&print.side_heavy)
			&& (0.0..=MAX_SIDE_WEIGHT).contains(&print.side_drop),
		"{print:?}"
	);
	assert!(
		(-1.0..=1.0).contains(&print.side_from) && (print.side_from..=1.0).contains(&print.side_to),
		"{print:?}"
	);
	assert!(
		(0.0..=MAX_WOBBLE_FREQUENCY).contains(&print.wobble_frequency),
		"{print:?}"
	);
}

fn assert_print_override_sanitized(value: f32) {
	let unit = |v: f32| (0.0..=1.0).contains(&v);
	let Op::SetPrint { target, print, .. } = (Op::SetPrint {
		node: NodeId(1),
		target: PrintTarget::Material("é".repeat(MAX_NAME_BYTES)),
		print: PrintOverride {
			lit: Some([value; 3]),
			shade: Some([value; 3]),
			deep: Some([value; 3]),
			spot: Some([value; 3]),
			mode: Some(PrintMode::Flat),
			hatch_min: Some(value),
			hatch_per_m: Some(value),
			cross_hatch: Some(true),
			specular: Some(value),
			rim: Some(true),
			ink_weight: Some(value),
			knockout: Some(false),
			boil_step: Some(254),
		},
	})
	.sanitized() else {
		unreachable!()
	};
	let PrintTarget::Material(material) = target else {
		unreachable!()
	};
	assert!(material.len() <= MAX_NAME_BYTES);
	assert!(print.lit.unwrap().into_iter().all(unit));
	assert!(print.shade.unwrap().into_iter().all(unit));
	assert!(print.deep.unwrap().into_iter().all(unit));
	assert!(print.spot.unwrap().into_iter().all(unit));
	assert!(unit(print.hatch_min.unwrap()));
	assert!((0.0..=MAX_HATCH_PER_M).contains(&print.hatch_per_m.unwrap()));
	assert!(unit(print.specular.unwrap()));
	assert!((0.0..=MAX_INK_WEIGHT).contains(&print.ink_weight.unwrap()));
	assert_eq!(print.boil_step, Some(2));
}

#[test]
fn sanitizing_bounds_print_and_camera_light() {
	for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0e30, -1.0e30] {
		assert_print_sanitized(value);
		assert_print_override_sanitized(value);

		let Op::SetLight(light) = Op::SetLight(Light {
			direction: [value; 3],
			color: [value; 3],
			intensity: value,
			ambient: value,
			shadow: value,
			camera_space: true,
		})
		.sanitized() else {
			unreachable!()
		};
		assert!(light.camera_space);
	}
}

#[test]
fn print_and_camera_light_are_new_ops_that_leave_old_batches_alone() {
	// A world-space light still encodes with tag 16 and the exact same bytes.
	let world = encode_ops(&[Op::SetLight(Light::default())]);
	assert_eq!(world[9], 16);
	assert_eq!(world.len(), 9 + 1 + 9 * 4);

	// A camera-space light uses tag 29 with the exact same payload after the
	// tag.
	let camera = encode_ops(&[Op::SetLight(Light {
		camera_space: true,
		..Light::default()
	})]);
	assert_eq!(camera[9], 29);
	assert_eq!(&camera[10..], &world[10..]);

	// Print style uses tag 27; PBR and Toon keep tag 23.
	let print_style = encode_ops(&[Op::SetStyle(Style::print())]);
	assert_eq!(print_style[9], 27);
	let toon_style = encode_ops(&[Op::SetStyle(Style::hand_drawn())]);
	assert_eq!(toon_style[9], 23);

	// SetPrint uses tag 28, node 7, target All (0), 13 absent option bytes.
	let print_op = encode_ops(&[Op::SetPrint {
		node: NodeId(7),
		target: PrintTarget::All,
		print: PrintOverride::default(),
	}]);
	assert_eq!(&print_op[9..15], &[28, 7, 0, 0, 0, 0]);
	assert_eq!(&print_op[15..], &[0; 13]);
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
			// The rotation is renormalized, to within a rounding.
			(
				Op::EaseTransform {
					transform: asked,
					seconds,
					easing,
					..
				},
				Op::EaseTransform {
					transform: kept,
					seconds: kept_seconds,
					easing: kept_easing,
					..
				},
			) => {
				assert_eq!(kept.translation, asked.translation);
				assert_eq!(kept.scale, asked.scale);
				assert!(
					kept.rotation
						.iter()
						.zip(asked.rotation)
						.all(|(kept, asked)| (kept - asked).abs() < 1.0e-6)
				);
				assert_eq!((kept_seconds, kept_easing), (seconds, easing));
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

#[test]
fn short_ops_round_trip_in_repeated_batches() {
	let styles = vec![Op::SetStyle(Style::default()); 3];
	assert_eq!(decode_ops(&encode_ops(&styles)), Ok(styles));

	let backdrops = vec![Op::SetBackdrop(Backdrop::Transparent); 4];
	assert_eq!(decode_ops(&encode_ops(&backdrops)), Ok(backdrops));
}

#[test]
fn easing_apply_handles_nan_and_infinities() {
	for easing in [Easing::Linear, Easing::Smooth, Easing::In, Easing::Out] {
		assert_eq!(easing.apply(f32::NAN), 0.0, "{easing:?}.apply(NaN)");
		assert_eq!(easing.apply(f32::NEG_INFINITY), 0.0, "{easing:?}(-inf)");
		assert_eq!(easing.apply(f32::INFINITY), 1.0, "{easing:?}(+inf)");
	}
}
