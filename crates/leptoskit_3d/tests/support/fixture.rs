//! Builds small glTF binaries for tests and benchmarks: a rig with a skinned
//! body, a morphing head, a socket, named parts, materials, and clips.
//!
//! Everything is in metres. The rig stands on y = 0, about 2 m tall, facing
//! +z.

#![allow(dead_code)]

use std::fmt::Write as _;

/// Accumulates a binary buffer and the JSON that describes it.
#[derive(Default)]
pub struct Builder {
	bin: Vec<u8>,
	views: Vec<String>,
	accessors: Vec<String>,
}

impl Builder {
	fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
		while self.bin.len() % 4 != 0 {
			self.bin.push(0);
		}

		let offset = self.bin.len();
		self.bin.extend_from_slice(bytes);
		let target = target.map_or(String::new(), |target| format!(r#","target":{target}"#));
		self.views.push(format!(
			r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{}{target}}}"#,
			bytes.len()
		));
		self.views.len() - 1
	}

	/// A float accessor, `width` floats per element.
	pub fn floats(&mut self, values: &[f32], width: usize, bounds: bool) -> usize {
		let bytes: Vec<u8> = values
			.iter()
			.flat_map(|value| value.to_le_bytes())
			.collect();
		let view = self.view(&bytes, None);

		let kind = match width {
			1 => "SCALAR",
			2 => "VEC2",
			3 => "VEC3",
			4 => "VEC4",
			16 => "MAT4",
			_ => unreachable!(),
		};

		let mut extra = String::new();

		if bounds {
			let mut min = vec![f32::INFINITY; width];
			let mut max = vec![f32::NEG_INFINITY; width];

			for chunk in values.chunks_exact(width) {
				for axis in 0..width {
					min[axis] = min[axis].min(chunk[axis]);
					max[axis] = max[axis].max(chunk[axis]);
				}
			}

			write!(extra, r#","min":{min:?},"max":{max:?}"#).unwrap();
		}

		self.accessors.push(format!(
			r#"{{"bufferView":{view},"componentType":5126,"count":{},"type":"{kind}"{extra}}}"#,
			values.len() / width
		));
		self.accessors.len() - 1
	}

	/// An accessor of unsigned 16-bit integers, `width` per element.
	pub fn u16s(&mut self, values: &[u16], width: usize) -> usize {
		let bytes: Vec<u8> = values
			.iter()
			.flat_map(|value| value.to_le_bytes())
			.collect();
		let view = self.view(&bytes, None);
		let kind = if width == 1 { "SCALAR" } else { "VEC4" };
		self.accessors.push(format!(
			r#"{{"bufferView":{view},"componentType":5123,"count":{},"type":"{kind}"}}"#,
			values.len() / width
		));
		self.accessors.len() - 1
	}

	/// Wraps `json` (the document without `buffers`, `bufferViews`, and
	/// `accessors`) and the buffer into a `.glb`.
	pub fn glb(self, json: &str) -> Vec<u8> {
		let json = format!(
			r#"{{{json},"buffers":[{{"byteLength":{}}}],"bufferViews":[{}],"accessors":[{}]}}"#,
			self.bin.len(),
			self.views.join(","),
			self.accessors.join(","),
		);
		let mut json = json.into_bytes();

		while json.len() % 4 != 0 {
			json.push(b' ');
		}

		let mut bin = self.bin;

		while bin.len() % 4 != 0 {
			bin.push(0);
		}

		let total = 12 + 8 + json.len() + 8 + bin.len();

		let mut glb = Vec::with_capacity(total);
		glb.extend_from_slice(b"glTF");
		glb.extend_from_slice(&2_u32.to_le_bytes());
		glb.extend_from_slice(&(total as u32).to_le_bytes());
		glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
		glb.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes());
		glb.extend_from_slice(&json);
		glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());

		glb.extend_from_slice(&0x004E_4942_u32.to_le_bytes());
		glb.extend_from_slice(&bin);
		glb
	}
}

/// An axis-aligned box's 24 vertices (4 per face, so normals stay flat) and
/// 36 indices.
pub fn cuboid(min: [f32; 3], max: [f32; 3]) -> (Vec<f32>, Vec<f32>, Vec<u16>) {
	let mut positions = Vec::new();
	let mut normals = Vec::new();
	let mut indices = Vec::new();
	let faces: [([f32; 3], [usize; 3]); 6] = [
		([1.0, 0.0, 0.0], [0, 1, 2]),
		([-1.0, 0.0, 0.0], [0, 2, 1]),
		([0.0, 1.0, 0.0], [1, 2, 0]),
		([0.0, -1.0, 0.0], [1, 0, 2]),
		([0.0, 0.0, 1.0], [2, 0, 1]),
		([0.0, 0.0, -1.0], [2, 1, 0]),
	];

	for (normal, [axis, u, v]) in faces {
		let side = normal[axis];
		let base = (positions.len() / 3) as u16;
		let corner = |a: f32, b: f32| {
			let mut point = [0.0; 3];
			point[axis] = if side > 0.0 { max[axis] } else { min[axis] };
			point[u] = if a > 0.0 { max[u] } else { min[u] };
			point[v] = if b > 0.0 { max[v] } else { min[v] };
			point
		};

		for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
			positions.extend_from_slice(&corner(a, b));
			normals.extend_from_slice(&normal);
		}

		indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
	}

	(positions, normals, indices)
}

/// The test rig:
///
/// - `rig`, the root, with children `hip` (a joint), `body` (a skinned box from
///   y = 0 to 1.6, its top half following `arm`), `head` (a box at y 1.6 to 2.0
///   with morph targets `grow` and `shrink`), and `socket` (at x = 0.5, y = 1).
/// - `hip` has a child joint `arm` at y = 0.8.
/// - Clips: `raise` turns `arm` 90° about z over 1 s, `bob` moves `rig` up by
///   0.5 m in steps, `swell` drives the head's weights, and `nod` rotates
///   `head` with a cubic spline.
/// - Materials: `paint` (opaque, red) on the body, `glass` (blended, blue, half
///   alpha) on the head.
pub fn rig() -> Vec<u8> {
	let mut builder = Builder::default();
	let (body_positions, body_normals, body_indices) = cuboid([-0.3, 0.0, -0.2], [0.3, 1.6, 0.2]);
	let body_position = builder.floats(&body_positions, 3, true);
	let body_normal = builder.floats(&body_normals, 3, false);
	let body_index = builder.u16s(&body_indices, 1);
	let vertices = body_positions.len() / 3;
	let mut joints = Vec::new();
	let mut weights = Vec::new();

	for vertex in 0..vertices {
		let upper = body_positions[vertex * 3 + 1] > 0.8;
		joints.extend_from_slice(&[u16::from(upper), 0, 0, 0]);
		weights.extend_from_slice(&[1.0, 0.0, 0.0, 0.0]);
	}

	let body_joints = builder.u16s(&joints, 4);
	let body_weights = builder.floats(&weights, 4, false);
	// The joints' inverse bind matrices: hip at the origin, arm at y = 0.8.
	#[rustfmt::skip]
	let inverse_bind: Vec<f32> = vec![
		1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
		1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -0.8, 0.0, 1.0,
	];
	let inverse_bind = builder.floats(&inverse_bind, 16, false);

	let (head_positions, head_normals, head_indices) = cuboid([-0.2, 1.6, -0.2], [0.2, 2.0, 0.2]);
	let head_position = builder.floats(&head_positions, 3, true);
	let head_normal = builder.floats(&head_normals, 3, false);
	let head_index = builder.u16s(&head_indices, 1);
	// `grow` pushes every vertex out by half again; `shrink` pulls it in.
	let grow: Vec<f32> = head_positions
		.as_chunks::<3>()
		.0
		.iter()
		.flat_map(|point| [point[0] * 0.5, 0.0, point[2] * 0.5])
		.collect();
	let shrink: Vec<f32> = grow.iter().map(|value| -value).collect();
	let grow = builder.floats(&grow, 3, true);
	let shrink = builder.floats(&shrink, 3, true);

	let times = builder.floats(&[0.0, 1.0], 1, true);
	let half = std::f32::consts::FRAC_1_SQRT_2;
	let raise = builder.floats(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, half, half], 4, false);
	let bob = builder.floats(&[0.0, 0.0, 0.0, 0.0, 0.5, 0.0], 3, false);
	let swell = builder.floats(&[0.0, 0.0, 1.0, 0.0], 2, false);
	let sin = (std::f32::consts::FRAC_PI_8).sin();
	let cos = (std::f32::consts::FRAC_PI_8).cos();
	#[rustfmt::skip]

	let nod = builder.floats(&[
		0.0, 0.0, 0.0, 0.0,  0.0, 0.0, 0.0, 1.0,  0.0, 0.0, 0.0, 0.0,
		0.0, 0.0, 0.0, 0.0,  sin, 0.0, 0.0, cos,  0.0, 0.0, 0.0, 0.0,
	], 4, false);

	let json = format!(
		r#""asset":{{"version":"2.0","generator":"leptoskit_3d tests"}},
"scene":0,"scenes":[{{"nodes":[0]}}],
"nodes":[
 {{"name":"rig","children":[1,3,4,5]}},
 {{"name":"hip","children":[2]}},
 {{"name":"arm","translation":[0,0.8,0]}},
 {{"name":"body","mesh":0,"skin":0}},
 {{"name":"head","mesh":1}},
 {{"name":"socket","translation":[0.5,1,0]}}
],
"skins":[{{"joints":[1,2],"inverseBindMatrices":{inverse_bind}}}],
"meshes":[
 {{"name":"body","primitives":[{{"attributes":{{"POSITION":{body_position},"NORMAL":{body_normal},"JOINTS_0":{body_joints},"WEIGHTS_0":{body_weights}}},"indices":{body_index},"material":0}}]}},
 {{"name":"head","primitives":[{{"attributes":{{"POSITION":{head_position},"NORMAL":{head_normal}}},"indices":{head_index},"material":1,
   "targets":[{{"POSITION":{grow}}},{{"POSITION":{shrink}}}]}}],
   "weights":[0,0],"extras":{{"targetNames":["grow","shrink"]}}}}
],
"materials":[
 {{"name":"paint","pbrMetallicRoughness":{{"baseColorFactor":[0.8,0.05,0.05,1],"metallicFactor":0,"roughnessFactor":0.6}}}},
 {{"name":"glass","pbrMetallicRoughness":{{"baseColorFactor":[0.1,0.2,0.9,0.5],"metallicFactor":0,"roughnessFactor":0.3}},"alphaMode":"BLEND","doubleSided":true}}
],
"animations":[
 {{"name":"raise","channels":[{{"sampler":0,"target":{{"node":2,"path":"rotation"}}}}],"samplers":[{{"input":{times},"output":{raise}}}]}},
 {{"name":"bob","channels":[{{"sampler":0,"target":{{"node":0,"path":"translation"}}}}],"samplers":[{{"input":{times},"output":{bob},"interpolation":"STEP"}}]}},
 {{"name":"swell","channels":[{{"sampler":0,"target":{{"node":4,"path":"weights"}}}}],"samplers":[{{"input":{times},"output":{swell}}}]}},
 {{"name":"nod","channels":[{{"sampler":0,"target":{{"node":4,"path":"rotation"}}}}],"samplers":[{{"input":{times},"output":{nod},"interpolation":"CUBICSPLINE"}}]}}
]"#
	);
	builder.glb(&json)
}

/// A unit cube from -0.5 to 0.5, white and unlit, for pixel tests that need
/// exact colours.
pub fn unlit_cube(color: [f32; 4], blend: bool) -> Vec<u8> {
	let mut builder = Builder::default();
	let (positions, normals, indices) = cuboid([-0.5; 3], [0.5; 3]);
	let position = builder.floats(&positions, 3, true);
	let normal = builder.floats(&normals, 3, false);
	let index = builder.u16s(&indices, 1);
	let alpha = if blend { r#","alphaMode":"BLEND""# } else { "" };
	let json = format!(
		r#""asset":{{"version":"2.0"}},"extensionsUsed":["KHR_materials_unlit"],
"scenes":[{{"nodes":[0]}}],
"nodes":[{{"name":"cube","mesh":0}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":{position},"NORMAL":{normal}}},"indices":{index},"material":0}}]}}],
"materials":[{{"name":"flat","pbrMetallicRoughness":{{"baseColorFactor":{color:?}}}{alpha},"extensions":{{"KHR_materials_unlit":{{}}}}}}]"#
	);
	builder.glb(&json)
}

/// A lit grey cube, for pixel tests of lighting.
pub fn lit_cube() -> Vec<u8> {
	let mut builder = Builder::default();
	let (positions, normals, indices) = cuboid([-0.5; 3], [0.5; 3]);
	let position = builder.floats(&positions, 3, true);
	let normal = builder.floats(&normals, 3, false);
	let index = builder.u16s(&indices, 1);
	let json = format!(
		r#""asset":{{"version":"2.0"}},
"scenes":[{{"nodes":[0]}}],
"nodes":[{{"name":"cube","mesh":0}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":{position},"NORMAL":{normal}}},"indices":{index},"material":0}}]}}],
"materials":[{{"name":"grey","pbrMetallicRoughness":{{"baseColorFactor":[0.5,0.5,0.5,1],"metallicFactor":0,"roughnessFactor":0.7}}}}]"#
	);
	builder.glb(&json)
}

/// A skinned character standing in for a game's: a tube 2 m tall of
/// `joints` rings of 64 vertices (with `joints = 40`, 10 240 vertices and
/// 20 000 triangles), each ring bound to a chain of joints, two morph
/// targets, a looping `sway` clip that turns every joint, and `spin`, which
/// sways the same way while turning the whole character once every four
/// seconds.
pub fn character(joints: usize) -> Vec<u8> {
	let mut builder = Builder::default();
	let around = 64;
	let rings = joints * 4;
	let height = 2.0;
	let mut positions = Vec::new();
	let mut normals = Vec::new();
	let mut joint_indices = Vec::new();
	let mut weights = Vec::new();

	let mut wide = Vec::new();

	for ring in 0..rings {
		let y = ring as f32 / (rings - 1) as f32 * height;
		let radius = 0.25 + 0.05 * (y * 6.0).sin();
		let segment = (ring * joints / rings).min(joints - 1);
		let next = (segment + 1).min(joints - 1);
		let blend = (ring * joints) as f32 / rings as f32 - segment as f32;

		for step in 0..around {
			let angle = step as f32 / around as f32 * std::f32::consts::TAU;
			let (sin, cos) = angle.sin_cos();
			positions.extend_from_slice(&[cos * radius, y, sin * radius]);
			normals.extend_from_slice(&[cos, 0.0, sin]);
			joint_indices.extend_from_slice(&[segment as u16, next as u16, 0, 0]);
			weights.extend_from_slice(&[1.0 - blend, blend, 0.0, 0.0]);
			wide.extend_from_slice(&[cos * 0.1, 0.0, sin * 0.1]);
		}
	}

	let mut indices = Vec::new();

	for ring in 0..rings - 1 {
		for step in 0..around {
			let a = (ring * around + step) as u16;
			let b = (ring * around + (step + 1) % around) as u16;
			let c = a + around as u16;
			let d = b + around as u16;
			indices.extend_from_slice(&[a, c, b, b, c, d]);
		}
	}

	let tall: Vec<f32> = positions
		.as_chunks::<3>()
		.0
		.iter()
		.flat_map(|point| [0.0, point[1] * 0.1, 0.0])
		.collect();
	let position = builder.floats(&positions, 3, true);
	let normal = builder.floats(&normals, 3, false);
	let joint = builder.u16s(&joint_indices, 4);
	let weight = builder.floats(&weights, 4, false);
	let index = builder.u16s(&indices, 1);
	let wide = builder.floats(&wide, 3, true);
	let tall = builder.floats(&tall, 3, true);

	let spacing = height / joints as f32;
	let inverse_bind: Vec<f32> = (0..joints)
		.flat_map(|joint| {
			let mut matrix = [0.0; 16];
			matrix[0] = 1.0;
			matrix[5] = 1.0;
			matrix[10] = 1.0;
			matrix[15] = 1.0;
			matrix[13] = -(joint as f32) * spacing;
			matrix
		})
		.collect();
	let inverse_bind = builder.floats(&inverse_bind, 16, false);
	let keys = 31;
	let times: Vec<f32> = (0..keys).map(|key| key as f32 / 30.0).collect();
	let times = builder.floats(&times, 1, true);
	// The root holds the skinned mesh and the chain's first joint.
	let mut nodes = vec![
		r#"{"name":"character","children":[1,2]}"#.to_owned(),
		r#"{"name":"skin","mesh":0,"skin":0}"#.to_owned(),
	];
	let mut channels = Vec::new();
	let mut samplers = Vec::new();

	for joint in 0..joints {
		let children = if joint + 1 < joints {
			format!(r#","children":[{}]"#, joint + 3)
		} else {
			String::new()
		};
		let y = if joint == 0 { 0.0 } else { spacing };
		nodes.push(format!(
			r#"{{"name":"joint{joint}","translation":[0,{y},0]{children}}}"#
		));
		let rotations: Vec<f32> = (0..keys)
			.flat_map(|key| {
				let phase = key as f32 / 30.0 * std::f32::consts::TAU + joint as f32 * 0.3;
				let half = phase.sin() * 0.04;
				[0.0, 0.0, half.sin(), half.cos()]
			})
			.collect();

		let output = builder.floats(&rotations, 4, false);
		samplers.push(format!(r#"{{"input":{times},"output":{output}}}"#));
		channels.push(format!(
			r#"{{"sampler":{joint},"target":{{"node":{},"path":"rotation"}}}}"#,
			joint + 2
		));
	}

	// `spin` turns the whole character once around +y every four seconds while
	// it sways, so one clip shows both: clips replace each other rather than
	// layer.
	let spin_keys = 121;
	let spin_times: Vec<f32> = (0..spin_keys).map(|key| key as f32 / 30.0).collect();
	let spin_times = builder.floats(&spin_times, 1, true);
	let turn: Vec<f32> = (0..spin_keys)
		.flat_map(|key| {
			let half = key as f32 / 30.0 * std::f32::consts::FRAC_PI_4;
			[0.0, half.sin(), 0.0, half.cos()]
		})
		.collect();
	let turn = builder.floats(&turn, 4, false);
	let mut spin_channels =
		vec![r#"{"sampler":0,"target":{"node":0,"path":"rotation"}}"#.to_owned()];
	let mut spin_samplers = vec![format!(r#"{{"input":{spin_times},"output":{turn}}}"#)];

	for joint in 0..joints {
		let rotations: Vec<f32> = (0..spin_keys)
			.flat_map(|key| {
				let phase = key as f32 / 30.0 * std::f32::consts::TAU + joint as f32 * 0.3;
				let half = phase.sin() * 0.04;
				[0.0, 0.0, half.sin(), half.cos()]
			})
			.collect();
		let output = builder.floats(&rotations, 4, false);
		spin_samplers.push(format!(r#"{{"input":{spin_times},"output":{output}}}"#));
		spin_channels.push(format!(
			r#"{{"sampler":{},"target":{{"node":{},"path":"rotation"}}}}"#,
			joint + 1,
			joint + 2
		));
	}

	let joint_nodes: Vec<String> = (0..joints).map(|joint| (joint + 2).to_string()).collect();
	let json = format!(
		r#""asset":{{"version":"2.0"}},
"scenes":[{{"nodes":[0]}}],
"nodes":[{}],
"skins":[{{"joints":[{}],"inverseBindMatrices":{inverse_bind}}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":{position},"NORMAL":{normal},"JOINTS_0":{joint},"WEIGHTS_0":{weight}}},"indices":{index},"material":0,
  "targets":[{{"POSITION":{wide}}},{{"POSITION":{tall}}}]}}],"weights":[0.5,0],"extras":{{"targetNames":["wide","tall"]}}}}],
"materials":[{{"pbrMetallicRoughness":{{"baseColorFactor":[0.6,0.5,0.4,1],"metallicFactor":0,"roughnessFactor":0.7}}}}],
"animations":[
 {{"name":"sway","channels":[{}],"samplers":[{}]}},
 {{"name":"spin","channels":[{}],"samplers":[{}]}}
]"#,
		nodes.join(","),
		joint_nodes.join(","),
		channels.join(","),
		samplers.join(","),
		spin_channels.join(","),
		spin_samplers.join(","),
	);
	builder.glb(&json)
}
