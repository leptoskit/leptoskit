//! Bounds for ops from code the host does not trust, such as a Wasm guest,
//! and for the scenes and files they build. The host sanitizes every op
//! before applying it.

use crate::protocol::*;

/// The largest encoded batch of ops, in bytes.
pub const MAX_BATCH_BYTES: usize = 1 << 20;
/// The most ops (or events, or hits) in one batch.
pub const MAX_OPS: usize = 16_384;
/// The longest part, clip, morph target, or bone name, in bytes.
pub const MAX_NAME_BYTES: usize = 256;
/// The longest asset path, in bytes.
pub const MAX_PATH_BYTES: usize = 1024;
/// The most scenes one app holds at once.
pub const MAX_SCENES: usize = 16;
/// The most nodes a scene holds; later spawns are ignored.
pub const MAX_NODES: usize = 4096;
/// The most assets one scene refers to at once.
pub const MAX_ASSETS: usize = 64;
/// The most raycasts answered per batch; later ones are answered empty.
pub const MAX_RAYCASTS_PER_BATCH: usize = 64;
/// The most hits one raycast reports.
pub const MAX_HITS: usize = 32;
/// The most events waiting for the guest; past it, the oldest are dropped.
pub const MAX_PENDING_EVENTS: usize = 1024;
/// The largest model file, in bytes.
pub const MAX_ASSET_BYTES: usize = 128 << 20;
/// The most vertices one model may decode to, across its meshes.
pub const MAX_ASSET_VERTICES: usize = 4 << 20;
/// The most indices one model may decode to.
pub const MAX_ASSET_INDICES: usize = 24 << 20;
/// The most nodes one model may have.
pub const MAX_ASSET_NODES: usize = 16_384;
/// The most joints one skin may have.
pub const MAX_JOINTS: usize = 256;
/// The most morph targets one mesh primitive may have.
pub const MAX_MORPH_TARGETS: usize = 64;
/// The most animation clips one model may have.
pub const MAX_CLIPS: usize = 1024;
/// The most keyframes one model's animations may hold, across channels.
pub const MAX_KEYFRAMES: usize = 8 << 20;
/// The widest or tallest texture, in pixels.
pub const MAX_TEXTURE_SIZE: u32 = 4096;
/// The largest coordinate, in metres, either way from the origin.
pub const MAX_COORDINATE: f32 = 1.0e6;
/// The largest scale factor, either way.
pub const MAX_SCALE: f32 = 1.0e4;
/// The fastest playback rate, either way.
pub const MAX_SPEED: f32 = 100.0;
/// The longest fade, in seconds.
pub const MAX_FADE: f32 = 60.0;
/// The largest morph weight, either way.
pub const MAX_MORPH_WEIGHT: f32 = 16.0;
/// The brightest emissive strength or light intensity.
pub const MAX_INTENSITY: f32 = 1.0e5;
/// The strongest rim power.
pub const MAX_RIM_POWER: f32 = 64.0;
/// The largest exposure.
pub const MAX_EXPOSURE: f32 = 64.0;
/// The narrowest and widest vertical field of view, in radians.
pub const FOV_RANGE: (f32, f32) = (0.001_745, 3.124);
/// The nearest a camera's near plane may be, in metres.
pub const MIN_NEAR: f32 = 1.0e-4;
/// The farthest a camera's far plane may be, in metres.
pub const MAX_FAR: f32 = 1.0e7;

/// A finite value clamped to `[min, max]`; an infinity or NaN becomes the
/// bound closest to zero.
fn bounded(value: f32, min: f32, max: f32) -> f32 {
	if value.is_finite() {
		value.clamp(min, max)
	} else {
		0.0_f32.clamp(min, max)
	}
}

fn unit(value: f32) -> f32 {
	bounded(value, 0.0, 1.0)
}

fn coordinate(value: f32) -> f32 {
	bounded(value, -MAX_COORDINATE, MAX_COORDINATE)
}

fn coordinates<const N: usize>(values: [f32; N]) -> [f32; N] {
	values.map(coordinate)
}

fn color<const N: usize>(values: [f32; N]) -> [f32; N] {
	values.map(unit)
}

/// A unit-length vector, or `fallback` when `values` has no direction.
fn direction(values: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
	let values = coordinates(values);
	let length = values.iter().map(|value| value * value).sum::<f32>().sqrt();
	if length > 1.0e-12 && length.is_finite() {
		values.map(|value| value / length)
	} else {
		fallback
	}
}

/// Cuts `name` to `limit` bytes at a character boundary.
fn name(mut name: String, limit: usize) -> String {
	if name.len() > limit {
		let mut end = limit;
		while !name.is_char_boundary(end) {
			end -= 1;
		}
		name.truncate(end);
	}
	name
}

/// A finite matrix with every entry bounded, or the identity when it holds
/// a NaN or an infinity.
fn matrix(values: [f32; 16]) -> [f32; 16] {
	if values.iter().all(|value| value.is_finite()) {
		values.map(|value| value.clamp(-MAX_COORDINATE, MAX_COORDINATE))
	} else {
		let mut identity = [0.0; 16];
		for index in 0..4 {
			identity[index * 5] = 1.0;
		}
		identity
	}
}

impl Transform {
	/// Finite, with translation and scale bounded and a unit rotation (the
	/// identity when it has no length).
	#[must_use]
	pub fn sanitized(self) -> Self {
		let rotation = self.rotation.map(|value| bounded(value, -1.0e3, 1.0e3));
		let length = rotation
			.iter()
			.map(|value| value * value)
			.sum::<f32>()
			.sqrt();
		Self {
			translation: coordinates(self.translation),
			rotation: if length > 1.0e-12 {
				rotation.map(|value| value / length)
			} else {
				Self::IDENTITY.rotation
			},
			scale: self
				.scale
				.map(|value| bounded(value, -MAX_SCALE, MAX_SCALE)),
		}
	}
}

impl MaterialOverride {
	/// Colours between 0 and 1, strengths and powers bounded.
	#[must_use]
	pub fn sanitized(self) -> Self {
		Self {
			color: self.color.map(color),
			emissive: self.emissive.map(color),
			emissive_strength: self
				.emissive_strength
				.map(|value| bounded(value, 0.0, MAX_INTENSITY)),
			opacity: self.opacity.map(unit),
			rim: self.rim.map(|rim| {
				Rim {
					color: color(rim.color),
					strength: bounded(rim.strength, 0.0, MAX_INTENSITY),
					power: bounded(rim.power, 0.0, MAX_RIM_POWER),
				}
			}),
		}
	}
}

impl Light {
	/// A unit direction (straight down when it has none), colours between 0
	/// and 1, and bounded strengths.
	#[must_use]
	pub fn sanitized(self) -> Self {
		Self {
			direction: direction(self.direction, [0.0, -1.0, 0.0]),
			color: color(self.color),
			intensity: bounded(self.intensity, 0.0, MAX_INTENSITY),
			ambient: bounded(self.ambient, 0.0, MAX_INTENSITY),
			shadow: unit(self.shadow),
		}
	}
}

impl Op {
	/// The op with every number finite and bounded and every name within its
	/// limit, so applying it cannot make the host allocate or loop without
	/// limit.
	#[must_use]
	pub fn sanitized(self) -> Self {
		match self {
			Self::LoadAsset { asset, path } => {
				Self::LoadAsset {
					asset,
					path: name(path, MAX_PATH_BYTES),
				}
			}
			Self::SetTransform { node, transform } => {
				Self::SetTransform {
					node,
					transform: transform.sanitized(),
				}
			}
			Self::SetPartVisible {
				node,
				part,
				visible,
			} => {
				Self::SetPartVisible {
					node,
					part: name(part, MAX_NAME_BYTES),
					visible,
				}
			}
			Self::Play { node, play } => {
				Self::Play {
					node,
					play: Play {
						clip: name(play.clip, MAX_NAME_BYTES),
						speed: bounded(play.speed, -MAX_SPEED, MAX_SPEED),
						mode: play.mode,
						fade: bounded(play.fade, 0.0, MAX_FADE),
					},
				}
			}
			Self::Stop { node, fade } => {
				Self::Stop {
					node,
					fade: bounded(fade, 0.0, MAX_FADE),
				}
			}
			Self::SetMorphWeight {
				node,
				part,
				target,
				weight,
			} => {
				Self::SetMorphWeight {
					node,
					part: part.map(|part| name(part, MAX_NAME_BYTES)),
					target: name(target, MAX_NAME_BYTES),
					weight: bounded(weight, -MAX_MORPH_WEIGHT, MAX_MORPH_WEIGHT),
				}
			}
			Self::SetMaterial {
				node,
				part,
				material,
			} => {
				Self::SetMaterial {
					node,
					part: part.map(|part| name(part, MAX_NAME_BYTES)),
					material: material.sanitized(),
				}
			}
			Self::Attach { node, target, bone } => {
				Self::Attach {
					node,
					target,
					bone: name(bone, MAX_NAME_BYTES),
				}
			}
			Self::SetCamera {
				transform,
				fov_y,
				near,
				far,
			} => {
				let near = bounded(near, MIN_NEAR, MAX_FAR / 2.0);
				Self::SetCamera {
					transform: transform.sanitized(),
					fov_y: if fov_y.is_finite() {
						fov_y.clamp(FOV_RANGE.0, FOV_RANGE.1)
					} else {
						1.0
					},
					near,
					far: if far.is_finite() {
						far.clamp(near * 2.0, MAX_FAR)
					} else {
						MAX_FAR
					},
				}
			}
			Self::SetCameraMatrices { view, projection } => {
				Self::SetCameraMatrices {
					view: matrix(view),
					projection: matrix(projection),
				}
			}
			Self::SetLight(light) => Self::SetLight(light.sanitized()),
			Self::SetBackdrop(Backdrop::Color(value)) => {
				Self::SetBackdrop(Backdrop::Color(color(value)))
			}
			Self::SetExposure(exposure) => Self::SetExposure(bounded(exposure, 0.0, MAX_EXPOSURE)),
			Self::Raycast {
				request,
				origin,
				direction: towards,
				max_distance,
			} => {
				Self::Raycast {
					request,
					origin: coordinates(origin),
					direction: direction(towards, [0.0, 0.0, -1.0]),
					max_distance: bounded(max_distance, 0.0, MAX_FAR),
				}
			}
			Self::RaycastScreen { request, x, y } => {
				Self::RaycastScreen {
					request,
					x: unit(x),
					y: unit(y),
				}
			}
			op @ (Self::UnloadAsset { .. }
			| Self::Spawn { .. }
			| Self::Remove { .. }
			| Self::SetVisible { .. }
			| Self::SetShadow { .. }
			| Self::SetBackdrop(_)) => op,
		}
	}
}

/// Whether `path` is a safe relative asset path: not empty, no leading `/`,
/// no `\`, no NUL, no `.` or `..` segment, and no `:` (which Windows reads as
/// a drive or a stream).
pub fn is_safe_asset_path(path: &str) -> bool {
	!path.is_empty()
		&& path.len() <= MAX_PATH_BYTES
		&& !path.starts_with('/')
		&& !path.contains(['\\', '\0', ':'])
		&& path
			.split('/')
			.all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}
