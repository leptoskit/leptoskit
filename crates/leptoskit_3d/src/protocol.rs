//! The ops a guest sends to its host's scene, the events it gets back, and
//! their compact binary form.
//!
//! The host owns the scene. A guest never holds GPU objects or meshes: it
//! names nodes and assets with ids it picks, sends ops that change them, and
//! learns what happened from events. Ids are the guest's own, so a batch can
//! spawn a node and animate it without waiting for a reply.
//!
//! Every batch is little endian:
//!
//! ```text
//! batch  = "LK3D" version:u8 count:u32 op{count}
//! events = "LK3E" version:u8 count:u32 event{count}
//! str    = length:u16 utf8
//! opt<T> = 0 | 1 T
//! ```
//!
//! [`decode_ops`] and [`decode_events`] reject anything malformed: an unknown
//! tag, a string that is not UTF-8 or is longer than its limit, a count past
//! [`MAX_OPS`](crate::MAX_OPS), or bytes left over. Decoding does not bound
//! the values; [`Op::sanitized`] does that, and the host applies it to every
//! op.

use std::fmt;

/// The first bytes of an encoded batch of ops.
pub const OPS_MAGIC: &[u8; 4] = b"LK3D";
/// The first bytes of an encoded batch of events.
pub const EVENTS_MAGIC: &[u8; 4] = b"LK3E";
/// The version of the binary form this crate writes and reads.
pub const VERSION: u8 = 1;

macro_rules! id {
	($(#[$doc:meta])* $name:ident) => {
		$(#[$doc])*
		#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
		pub struct $name(pub u32);

		impl fmt::Display for $name {
			fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
				self.0.fmt(formatter)
			}
		}
	};
}

id!(
	/// A scene, chosen by the guest. A `Scene3d` view shows the scene with its
	/// id.
	SceneId
);
id!(
	/// A node in a scene: a model instance or an empty group, chosen by the
	/// guest.
	NodeId
);
id!(
	/// A loaded model file, chosen by the guest.
	AssetId
);
id!(
	/// Pairs a raycast with the event that answers it.
	RequestId
);

/// A node's place relative to its parent: translate, then rotate, then scale,
/// as glTF composes them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
	/// Metres along x, y (up), and z (towards the viewer).
	pub translation: [f32; 3],
	/// A unit quaternion, `[x, y, z, w]`.
	pub rotation: [f32; 4],
	/// A factor per axis.
	pub scale: [f32; 3],
}

impl Default for Transform {
	fn default() -> Self {
		Self::IDENTITY
	}
}

impl Transform {
	/// No translation, rotation, or scale.
	pub const IDENTITY: Self = Self {
		translation: [0.0; 3],
		rotation: [0.0, 0.0, 0.0, 1.0],
		scale: [1.0; 3],
	};

	/// A translation alone.
	pub const fn from_translation(x: f32, y: f32, z: f32) -> Self {
		Self {
			translation: [x, y, z],
			..Self::IDENTITY
		}
	}

	/// The same transform turned `radians` about the vertical axis,
	/// replacing its rotation.
	#[must_use]
	pub fn with_yaw(self, radians: f32) -> Self {
		let (sin, cos) = (radians * 0.5).sin_cos();
		Self {
			rotation: [0.0, sin, 0.0, cos],
			..self
		}
	}

	/// The same transform with its rotation replaced by `[x, y, z, w]`.
	#[must_use]
	pub fn with_rotation(self, rotation: [f32; 4]) -> Self {
		Self { rotation, ..self }
	}

	/// The same transform scaled evenly by `scale`, replacing its scale.
	#[must_use]
	pub fn with_scale(self, scale: f32) -> Self {
		Self {
			scale: [scale; 3],
			..self
		}
	}
}

/// What an animation clip does when it reaches its end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopMode {
	/// Plays once and holds its last pose. An
	/// [`Event::AnimationFinished`] follows.
	Once,
	/// Starts again from the beginning.
	#[default]
	Loop,
	/// Plays forwards, then backwards, and again.
	PingPong,
}

/// Starts an animation clip on a model instance.
#[derive(Clone, Debug, PartialEq)]
pub struct Play {
	/// The clip's name in the model file.
	pub clip: String,
	/// Playback rate: 1 is as authored, 0.5 half speed, -1 backwards.
	pub speed: f32,
	/// What happens at the clip's end.
	pub mode: LoopMode,
	/// Seconds over which the clip fades in while the one playing fades out.
	/// Zero cuts straight to it.
	pub fade: f32,
}

impl Play {
	/// The clip at its authored speed, looping, cut in without a fade.
	pub fn new(clip: impl Into<String>) -> Self {
		Self {
			clip: clip.into(),
			speed: 1.0,
			mode: LoopMode::Loop,
			fade: 0.0,
		}
	}

	/// Plays at `speed` times the authored rate.
	#[must_use]
	pub fn speed(mut self, speed: f32) -> Self {
		self.speed = speed;
		self
	}

	/// Sets what happens at the clip's end.
	#[must_use]
	pub fn mode(mut self, mode: LoopMode) -> Self {
		self.mode = mode;
		self
	}

	/// Plays once and holds the last pose.
	#[must_use]
	pub fn once(self) -> Self {
		self.mode(LoopMode::Once)
	}

	/// Crossfades from the clip playing over `seconds`.
	#[must_use]
	pub fn fade(mut self, seconds: f32) -> Self {
		self.fade = seconds;
		self
	}
}

/// A Fresnel rim: a glow along the silhouette, strongest where the surface
/// turns away from the viewer. Cheap, and it reads well over a camera image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rim {
	/// Linear RGB, each from 0 to 1.
	pub color: [f32; 3],
	/// How bright the rim is; 0 turns it off.
	pub strength: f32,
	/// How tight the rim hugs the silhouette; higher is thinner.
	pub power: f32,
}

/// Changes to a model's materials, applied on top of what the file says.
/// A field left `None` keeps its current value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MaterialOverride {
	/// Multiplies the base colour, RGBA in linear light.
	pub color: Option<[f32; 4]>,
	/// Replaces the emissive colour, linear RGB.
	pub emissive: Option<[f32; 3]>,
	/// Multiplies the emissive colour, as `KHR_materials_emissive_strength`
	/// does.
	pub emissive_strength: Option<f32>,
	/// Multiplies alpha. Below 1 the part draws with the transparent parts,
	/// sorted back to front.
	pub opacity: Option<f32>,
	/// A Fresnel rim.
	pub rim: Option<Rim>,
}

/// The directional light, standing in for the sun or a room's key light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
	/// The direction the light travels, towards the scene. Need not be unit
	/// length.
	pub direction: [f32; 3],
	/// Linear RGB.
	pub color: [f32; 3],
	/// Illuminance; 3 is a bright interior.
	pub intensity: f32,
	/// How strongly the environment map lights the scene; 1 is as baked.
	pub ambient: f32,
	/// The darkness of the soft shadow under each model, from 0 to 1.
	pub shadow: f32,
}

impl Default for Light {
	fn default() -> Self {
		Self {
			direction: [-0.4, -1.0, -0.6],
			color: [1.0, 0.97, 0.92],
			intensity: 3.0,
			ambient: 1.0,
			shadow: 0.5,
		}
	}
}

/// What the scene draws behind its models. A host drawing for augmented
/// reality or a headset may pass its own background instead.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Backdrop {
	/// Nothing, so the view shows what is behind it.
	#[default]
	Transparent,
	/// A solid colour, linear RGBA.
	Color([f32; 4]),
	/// The device camera, when the host provides one (augmented reality).
	/// Elsewhere it draws as [`Backdrop::Transparent`].
	Camera,
}

/// A change to a scene.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
	/// Loads a model file from the app's assets, by a relative path such as
	/// `models/reaper.glb`. [`Event::AssetLoaded`] or [`Event::AssetFailed`]
	/// follows. Loading the same path twice shares one copy.
	LoadAsset {
		/// The id the guest gives the asset.
		asset: AssetId,
		/// A relative path inside the app's assets, with `/` separators.
		path: String,
	},
	/// Forgets an asset. Instances already spawned from it keep drawing.
	UnloadAsset {
		/// The asset to forget.
		asset: AssetId,
	},
	/// Adds a node: an instance of `asset`, or an empty group when `asset` is
	/// `None`. An instance of an asset that is still loading appears when it
	/// loads; ops sent to it meanwhile apply then.
	Spawn {
		/// The id the guest gives the node.
		node: NodeId,
		/// The model to instance.
		asset: Option<AssetId>,
		/// The node it moves with, or `None` for the scene's root.
		parent: Option<NodeId>,
	},
	/// Removes a node and every node under it.
	Remove {
		/// The node to remove.
		node: NodeId,
	},
	/// Places a node relative to its parent.
	SetTransform {
		/// The node to move.
		node: NodeId,
		/// Its new transform.
		transform: Transform,
	},
	/// Shows or hides a node and everything under it.
	SetVisible {
		/// The node.
		node: NodeId,
		/// Whether it draws.
		visible: bool,
	},
	/// Shows or hides a named part inside a model instance, such as one of
	/// several heads, and everything under it.
	SetPartVisible {
		/// The model instance.
		node: NodeId,
		/// The part's node name in the model file.
		part: String,
		/// Whether it draws.
		visible: bool,
	},
	/// Starts an animation clip.
	Play {
		/// The model instance.
		node: NodeId,
		/// The clip and how to play it.
		play: Play,
	},
	/// Stops the clip playing, fading back to the rest pose over `fade`
	/// seconds.
	Stop {
		/// The model instance.
		node: NodeId,
		/// Seconds to fade out; zero cuts.
		fade: f32,
	},
	/// Sets a morph target's weight. It replaces whatever a playing clip sets
	/// for that target, until the guest sets it again.
	SetMorphWeight {
		/// The model instance.
		node: NodeId,
		/// The part whose mesh has the target, or `None` for every mesh in the
		/// instance with a target of that name.
		part: Option<String>,
		/// The target's name, from the mesh's `extras.targetNames`.
		target: String,
		/// Usually between 0 and 1.
		weight: f32,
	},
	/// Changes how a model instance's materials draw.
	SetMaterial {
		/// The model instance.
		node: NodeId,
		/// The part to change, with everything under it, or `None` for the
		/// whole instance.
		part: Option<String>,
		/// What to change.
		material: MaterialOverride,
	},
	/// Turns the soft shadow under a model instance on or off. Instances cast
	/// one by default.
	SetShadow {
		/// The model instance.
		node: NodeId,
		/// Whether it casts a shadow.
		cast: bool,
	},
	/// Makes `node` move with a named part (a bone, a socket, any node) of
	/// the instance `target`, such as a weapon in a hand. An empty `bone`
	/// attaches it to the instance itself.
	Attach {
		/// The node to attach.
		node: NodeId,
		/// The model instance to attach it to.
		target: NodeId,
		/// The part's node name in `target`'s model file.
		bone: String,
	},
	/// Places a perspective camera in the scene.
	SetCamera {
		/// The camera's place; it looks along its own -z with +y up.
		transform: Transform,
		/// The vertical field of view, in radians.
		fov_y: f32,
		/// The nearest distance drawn, in metres.
		near: f32,
		/// The farthest distance drawn, in metres.
		far: f32,
	},
	/// Uses explicit view and projection matrices, column-major, such as the
	/// ones an augmented-reality session reports. A host rendering for a
	/// headset passes its own per eye instead.
	SetCameraMatrices {
		/// World to view.
		view: [f32; 16],
		/// View to clip space, with depth from 0 to 1.
		projection: [f32; 16],
	},
	/// Sets the directional light and the environment's strength.
	SetLight(Light),
	/// Sets what draws behind the models.
	SetBackdrop(Backdrop),
	/// Sets the exposure: 1 is neutral, 2 one stop brighter.
	SetExposure(f32),
	/// Casts a ray through the scene. [`Event::RaycastHits`] answers with
	/// `request`.
	Raycast {
		/// Echoed in the answer.
		request: RequestId,
		/// Where the ray starts, in world space.
		origin: [f32; 3],
		/// Where it points; need not be unit length.
		direction: [f32; 3],
		/// How far it reaches, in metres.
		max_distance: f32,
	},
	/// Casts a ray from the camera through a point of the view: `x` and `y`
	/// from 0 to 1, from the top-left corner. [`Event::RaycastHits`] answers
	/// with `request`.
	RaycastScreen {
		/// Echoed in the answer.
		request: RequestId,
		/// From 0 (left) to 1 (right).
		x: f32,
		/// From 0 (top) to 1 (bottom).
		y: f32,
	},
}

/// One place a ray met a model.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
	/// The model instance that was hit.
	pub node: NodeId,
	/// The name of the part that was hit, from the model file.
	pub part: String,
	/// Metres from the ray's origin.
	pub distance: f32,
	/// Where, in world space.
	pub point: [f32; 3],
	/// The surface's unit normal there, in world space.
	pub normal: [f32; 3],
}

/// Something the host tells the guest about its scene.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
	/// An asset loaded; instances of it now draw.
	AssetLoaded {
		/// The asset.
		asset: AssetId,
		/// Its animation clips' names.
		clips: Vec<String>,
	},
	/// An asset could not load.
	AssetFailed {
		/// The asset.
		asset: AssetId,
		/// Why, in English.
		message: String,
	},
	/// A clip played with [`LoopMode::Once`] reached its end.
	AnimationFinished {
		/// The model instance.
		node: NodeId,
		/// The clip.
		clip: String,
	},
	/// What a raycast met, nearest first.
	RaycastHits {
		/// The raycast's request.
		request: RequestId,
		/// The hits, nearest first; empty when it met nothing.
		hits: Vec<Hit>,
	},
}

/// Why a batch could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError(pub &'static str);

impl fmt::Display for DecodeError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(formatter, "invalid scene batch: {}", self.0)
	}
}

impl std::error::Error for DecodeError {}

// Op tags. Never reuse a retired number.
const LOAD_ASSET: u8 = 1;
const UNLOAD_ASSET: u8 = 2;
const SPAWN: u8 = 3;
const REMOVE: u8 = 4;
const SET_TRANSFORM: u8 = 5;
const SET_VISIBLE: u8 = 6;
const SET_PART_VISIBLE: u8 = 7;
const PLAY: u8 = 8;

const STOP: u8 = 9;
const SET_MORPH_WEIGHT: u8 = 10;
const SET_MATERIAL: u8 = 11;
const SET_SHADOW: u8 = 12;
const ATTACH: u8 = 13;
const SET_CAMERA: u8 = 14;
const SET_CAMERA_MATRICES: u8 = 15;
const SET_LIGHT: u8 = 16;

const SET_BACKDROP: u8 = 17;
const SET_EXPOSURE: u8 = 18;
const RAYCAST: u8 = 19;
const RAYCAST_SCREEN: u8 = 20;

// Event tags.
const ASSET_LOADED: u8 = 1;
const ASSET_FAILED: u8 = 2;
const ANIMATION_FINISHED: u8 = 3;
const RAYCAST_HITS: u8 = 4;

struct Writer(Vec<u8>);

impl Writer {
	fn u8(&mut self, value: u8) {
		self.0.push(value);
	}

	fn u16(&mut self, value: u16) {
		self.0.extend_from_slice(&value.to_le_bytes());
	}

	fn u32(&mut self, value: u32) {
		self.0.extend_from_slice(&value.to_le_bytes());
	}

	fn f32(&mut self, value: f32) {
		self.0.extend_from_slice(&value.to_le_bytes());
	}

	fn floats(&mut self, values: &[f32]) {
		for value in values {
			self.f32(*value);
		}
	}

	fn bool(&mut self, value: bool) {
		self.u8(u8::from(value));
	}

	/// A string, cut at a character boundary to what a u16 length holds.
	fn str(&mut self, value: &str) {
		let mut end = value.len().min(usize::from(u16::MAX));

		while !value.is_char_boundary(end) {
			end -= 1;
		}

		self.u16(end as u16);
		self.0.extend_from_slice(&value.as_bytes()[..end]);
	}

	fn option<T>(&mut self, value: Option<T>, write: impl FnOnce(&mut Self, T)) {
		match value {
			Some(value) => {
				self.u8(1);
				write(self, value);
			}
			None => self.u8(0),
		}
	}

	fn transform(&mut self, transform: &Transform) {
		self.floats(&transform.translation);
		self.floats(&transform.rotation);
		self.floats(&transform.scale);
	}
}

struct Reader<'a> {
	bytes: &'a [u8],
}

impl<'a> Reader<'a> {
	fn take(&mut self, count: usize) -> Result<&'a [u8], DecodeError> {
		if self.bytes.len() < count {
			return Err(DecodeError("it ends early"));
		}

		let (taken, rest) = self.bytes.split_at(count);
		self.bytes = rest;
		Ok(taken)
	}

	fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
		let mut array = [0; N];
		array.copy_from_slice(self.take(N)?);
		Ok(array)
	}

	fn u8(&mut self) -> Result<u8, DecodeError> {
		Ok(self.array::<1>()?[0])
	}

	fn u16(&mut self) -> Result<u16, DecodeError> {
		Ok(u16::from_le_bytes(self.array()?))
	}

	fn u32(&mut self) -> Result<u32, DecodeError> {
		Ok(u32::from_le_bytes(self.array()?))
	}

	fn f32(&mut self) -> Result<f32, DecodeError> {
		Ok(f32::from_le_bytes(self.array()?))
	}

	fn floats<const N: usize>(&mut self) -> Result<[f32; N], DecodeError> {
		let mut values = [0.0; N];

		for value in &mut values {
			*value = self.f32()?;
		}

		Ok(values)
	}

	fn bool(&mut self) -> Result<bool, DecodeError> {
		match self.u8()? {
			0 => Ok(false),
			1 => Ok(true),
			_ => Err(DecodeError("a flag is neither 0 nor 1")),
		}
	}

	fn str(&mut self, limit: usize) -> Result<String, DecodeError> {
		let length = usize::from(self.u16()?);

		if length > limit {
			return Err(DecodeError("a string is longer than its limit"));
		}

		std::str::from_utf8(self.take(length)?)
			.map(str::to_owned)
			.map_err(|_| DecodeError("a string is not UTF-8"))
	}

	fn option<T>(
		&mut self,
		read: impl FnOnce(&mut Self) -> Result<T, DecodeError>,
	) -> Result<Option<T>, DecodeError> {
		if self.bool()? {
			read(self).map(Some)
		} else {
			Ok(None)
		}
	}

	fn transform(&mut self) -> Result<Transform, DecodeError> {
		Ok(Transform {
			translation: self.floats()?,
			rotation: self.floats()?,
			scale: self.floats()?,
		})
	}

	fn header(&mut self, magic: [u8; 4]) -> Result<usize, DecodeError> {
		if self.take(4)? != magic {
			return Err(DecodeError("it does not start with the expected magic"));
		}

		if self.u8()? != VERSION {
			return Err(DecodeError("its version is not supported"));
		}

		let count = self.u32()? as usize;

		if count > crate::MAX_OPS {
			return Err(DecodeError("it holds more entries than the limit"));
		}

		Ok(count)
	}
}

fn write_material(writer: &mut Writer, material: &MaterialOverride) {
	writer.option(material.color, |writer, color| writer.floats(&color));
	writer.option(material.emissive, |writer, color| writer.floats(&color));
	writer.option(material.emissive_strength, Writer::f32);
	writer.option(material.opacity, Writer::f32);
	writer.option(material.rim, |writer, rim| {
		writer.floats(&rim.color);
		writer.f32(rim.strength);
		writer.f32(rim.power);
	});
}

fn read_material(reader: &mut Reader<'_>) -> Result<MaterialOverride, DecodeError> {
	Ok(MaterialOverride {
		color: reader.option(Reader::floats)?,
		emissive: reader.option(Reader::floats)?,
		emissive_strength: reader.option(Reader::f32)?,
		opacity: reader.option(Reader::f32)?,
		rim: reader.option(|reader| {
			Ok(Rim {
				color: reader.floats()?,
				strength: reader.f32()?,
				power: reader.f32()?,
			})
		})?,
	})
}

impl Op {
	fn write(&self, writer: &mut Writer) {
		match self {
			Self::LoadAsset { asset, path } => {
				writer.u8(LOAD_ASSET);
				writer.u32(asset.0);
				writer.str(path);
			}
			Self::UnloadAsset { asset } => {
				writer.u8(UNLOAD_ASSET);
				writer.u32(asset.0);
			}
			Self::Spawn {
				node,
				asset,
				parent,
			} => {
				writer.u8(SPAWN);
				writer.u32(node.0);
				writer.option(*asset, |writer, asset| writer.u32(asset.0));
				writer.option(*parent, |writer, parent| writer.u32(parent.0));
			}
			Self::Remove { node } => {
				writer.u8(REMOVE);
				writer.u32(node.0);
			}
			Self::SetTransform { node, transform } => {
				writer.u8(SET_TRANSFORM);
				writer.u32(node.0);
				writer.transform(transform);
			}
			Self::SetVisible { node, visible } => {
				writer.u8(SET_VISIBLE);
				writer.u32(node.0);
				writer.bool(*visible);
			}
			Self::SetPartVisible {
				node,
				part,
				visible,
			} => {
				writer.u8(SET_PART_VISIBLE);
				writer.u32(node.0);
				writer.str(part);
				writer.bool(*visible);
			}
			Self::Play { node, play } => {
				writer.u8(PLAY);
				writer.u32(node.0);
				writer.str(&play.clip);
				writer.f32(play.speed);
				writer.u8(play.mode as u8);
				writer.f32(play.fade);
			}
			Self::Stop { node, fade } => {
				writer.u8(STOP);
				writer.u32(node.0);
				writer.f32(*fade);
			}
			Self::SetMorphWeight {
				node,
				part,
				target,
				weight,
			} => {
				writer.u8(SET_MORPH_WEIGHT);
				writer.u32(node.0);
				writer.option(part.as_deref(), Writer::str);
				writer.str(target);
				writer.f32(*weight);
			}
			Self::SetMaterial {
				node,
				part,
				material,
			} => {
				writer.u8(SET_MATERIAL);
				writer.u32(node.0);
				writer.option(part.as_deref(), Writer::str);
				write_material(writer, material);
			}
			Self::SetShadow { node, cast } => {
				writer.u8(SET_SHADOW);
				writer.u32(node.0);
				writer.bool(*cast);
			}
			Self::Attach { node, target, bone } => {
				writer.u8(ATTACH);
				writer.u32(node.0);
				writer.u32(target.0);
				writer.str(bone);
			}
			Self::SetCamera {
				transform,
				fov_y,
				near,
				far,
			} => {
				writer.u8(SET_CAMERA);
				writer.transform(transform);
				writer.floats(&[*fov_y, *near, *far]);
			}
			Self::SetCameraMatrices { view, projection } => {
				writer.u8(SET_CAMERA_MATRICES);
				writer.floats(view);
				writer.floats(projection);
			}
			Self::SetLight(light) => {
				writer.u8(SET_LIGHT);
				writer.floats(&light.direction);
				writer.floats(&light.color);
				writer.floats(&[light.intensity, light.ambient, light.shadow]);
			}
			Self::SetBackdrop(backdrop) => {
				writer.u8(SET_BACKDROP);
				match backdrop {
					Backdrop::Transparent => writer.u8(0),
					Backdrop::Color(color) => {
						writer.u8(1);
						writer.floats(color);
					}
					Backdrop::Camera => writer.u8(2),
				}
			}
			Self::SetExposure(exposure) => {
				writer.u8(SET_EXPOSURE);
				writer.f32(*exposure);
			}
			Self::Raycast {
				request,
				origin,
				direction,
				max_distance,
			} => {
				writer.u8(RAYCAST);
				writer.u32(request.0);
				writer.floats(origin);
				writer.floats(direction);
				writer.f32(*max_distance);
			}
			Self::RaycastScreen { request, x, y } => {
				writer.u8(RAYCAST_SCREEN);
				writer.u32(request.0);
				writer.floats(&[*x, *y]);
			}
		}
	}

	#[allow(clippy::too_many_lines)]
	fn read(reader: &mut Reader<'_>) -> Result<Self, DecodeError> {
		use crate::MAX_NAME_BYTES as NAME;
		let tag = reader.u8()?;
		Ok(match tag {
			LOAD_ASSET => {
				Self::LoadAsset {
					asset: AssetId(reader.u32()?),
					path: reader.str(crate::MAX_PATH_BYTES)?,
				}
			}
			UNLOAD_ASSET => {
				Self::UnloadAsset {
					asset: AssetId(reader.u32()?),
				}
			}
			SPAWN => {
				Self::Spawn {
					node: NodeId(reader.u32()?),
					asset: reader.option(|reader| reader.u32().map(AssetId))?,
					parent: reader.option(|reader| reader.u32().map(NodeId))?,
				}
			}
			REMOVE => {
				Self::Remove {
					node: NodeId(reader.u32()?),
				}
			}
			SET_TRANSFORM => {
				Self::SetTransform {
					node: NodeId(reader.u32()?),
					transform: reader.transform()?,
				}
			}
			SET_VISIBLE => {
				Self::SetVisible {
					node: NodeId(reader.u32()?),
					visible: reader.bool()?,
				}
			}
			SET_PART_VISIBLE => {
				Self::SetPartVisible {
					node: NodeId(reader.u32()?),
					part: reader.str(NAME)?,
					visible: reader.bool()?,
				}
			}
			PLAY => {
				Self::Play {
					node: NodeId(reader.u32()?),
					play: Play {
						clip: reader.str(NAME)?,
						speed: reader.f32()?,
						mode: match reader.u8()? {
							0 => LoopMode::Once,
							1 => LoopMode::Loop,
							2 => LoopMode::PingPong,
							_ => return Err(DecodeError("an unknown loop mode")),
						},

						fade: reader.f32()?,
					},
				}
			}
			STOP => {
				Self::Stop {
					node: NodeId(reader.u32()?),
					fade: reader.f32()?,
				}
			}
			SET_MORPH_WEIGHT => {
				Self::SetMorphWeight {
					node: NodeId(reader.u32()?),
					part: reader.option(|reader| reader.str(NAME))?,
					target: reader.str(NAME)?,
					weight: reader.f32()?,
				}
			}
			SET_MATERIAL => {
				Self::SetMaterial {
					node: NodeId(reader.u32()?),
					part: reader.option(|reader| reader.str(NAME))?,
					material: read_material(reader)?,
				}
			}
			SET_SHADOW => {
				Self::SetShadow {
					node: NodeId(reader.u32()?),
					cast: reader.bool()?,
				}
			}
			ATTACH => {
				Self::Attach {
					node: NodeId(reader.u32()?),
					target: NodeId(reader.u32()?),
					bone: reader.str(NAME)?,
				}
			}
			SET_CAMERA => {
				let transform = reader.transform()?;
				let [fov_y, near, far] = reader.floats()?;
				Self::SetCamera {
					transform,
					fov_y,
					near,
					far,
				}
			}
			SET_CAMERA_MATRICES => {
				Self::SetCameraMatrices {
					view: reader.floats()?,
					projection: reader.floats()?,
				}
			}
			SET_LIGHT => {
				let direction = reader.floats()?;
				let color = reader.floats()?;
				let [intensity, ambient, shadow] = reader.floats()?;
				Self::SetLight(Light {
					direction,
					color,
					intensity,
					ambient,
					shadow,
				})
			}
			SET_BACKDROP => {
				Self::SetBackdrop(match reader.u8()? {
					0 => Backdrop::Transparent,
					1 => Backdrop::Color(reader.floats()?),
					2 => Backdrop::Camera,
					_ => return Err(DecodeError("an unknown background")),
				})
			}
			SET_EXPOSURE => Self::SetExposure(reader.f32()?),
			RAYCAST => {
				Self::Raycast {
					request: RequestId(reader.u32()?),
					origin: reader.floats()?,
					direction: reader.floats()?,
					max_distance: reader.f32()?,
				}
			}
			RAYCAST_SCREEN => {
				let request = RequestId(reader.u32()?);
				let [x, y] = reader.floats()?;
				Self::RaycastScreen { request, x, y }
			}
			_ => return Err(DecodeError("an unknown op")),
		})
	}
}

impl Event {
	/// Writes the event as its tag, then its fields in declaration order.
	fn write(&self, writer: &mut Writer) {
		match self {
			Self::AssetLoaded { asset, clips } => {
				writer.u8(ASSET_LOADED);
				writer.u32(asset.0);
				writer.u32(clips.len() as u32);
				for clip in clips {
					writer.str(clip);
				}
			}
			Self::AssetFailed { asset, message } => {
				writer.u8(ASSET_FAILED);
				writer.u32(asset.0);
				writer.str(message);
			}
			Self::AnimationFinished { node, clip } => {
				writer.u8(ANIMATION_FINISHED);
				writer.u32(node.0);
				writer.str(clip);
			}
			Self::RaycastHits { request, hits } => {
				writer.u8(RAYCAST_HITS);
				writer.u32(request.0);
				writer.u32(hits.len() as u32);
				for hit in hits {
					writer.u32(hit.node.0);
					writer.str(&hit.part);
					writer.f32(hit.distance);
					writer.floats(&hit.point);
					writer.floats(&hit.normal);
				}
			}
		}
	}

	fn read(reader: &mut Reader<'_>) -> Result<Self, DecodeError> {
		use crate::MAX_NAME_BYTES as NAME;
		let count = |reader: &mut Reader<'_>| {
			let count = reader.u32()? as usize;

			if count > crate::MAX_OPS {
				return Err(DecodeError("a list is longer than the limit"));
			}

			Ok(count)
		};
		Ok(match reader.u8()? {
			ASSET_LOADED => {
				let asset = AssetId(reader.u32()?);
				let clips = (0..count(reader)?)
					.map(|_| reader.str(NAME))
					.collect::<Result<_, _>>()?;
				Self::AssetLoaded { asset, clips }
			}
			ASSET_FAILED => {
				Self::AssetFailed {
					asset: AssetId(reader.u32()?),
					message: reader.str(usize::from(u16::MAX))?,
				}
			}
			ANIMATION_FINISHED => {
				Self::AnimationFinished {
					node: NodeId(reader.u32()?),
					clip: reader.str(NAME)?,
				}
			}
			RAYCAST_HITS => {
				let request = RequestId(reader.u32()?);
				let hits = (0..count(reader)?)
					.map(|_| {
						Ok(Hit {
							node: NodeId(reader.u32()?),
							part: reader.str(NAME)?,
							distance: reader.f32()?,
							point: reader.floats()?,
							normal: reader.floats()?,
						})
					})
					.collect::<Result<_, _>>()?;
				Self::RaycastHits { request, hits }
			}
			_ => return Err(DecodeError("an unknown event")),
		})
	}
}

/// Encodes a batch of ops for [`decode_ops`].
pub fn encode_ops(ops: &[Op]) -> Vec<u8> {
	let mut writer = Writer(Vec::with_capacity(9 + ops.len() * 32));
	writer.0.extend_from_slice(OPS_MAGIC);
	writer.u8(VERSION);
	writer.u32(ops.len() as u32);

	for op in ops {
		op.write(&mut writer);
	}

	writer.0
}

/// Decodes a batch written by [`encode_ops`]. Values are not bounded yet:
/// apply [`Op::sanitized`] to ops from code you do not trust.
pub fn decode_ops(bytes: &[u8]) -> Result<Vec<Op>, DecodeError> {
	if bytes.len() > crate::MAX_BATCH_BYTES {
		return Err(DecodeError("it is larger than the limit"));
	}

	let mut reader = Reader { bytes };
	let count = reader.header(*OPS_MAGIC)?;
	// Every op takes at least five bytes, so a count the bytes cannot hold
	// fails before allocating for it.
	if count > reader.bytes.len() / 5 + 1 {
		return Err(DecodeError("it ends early"));
	}

	let mut ops = Vec::with_capacity(count);

	for _ in 0..count {
		ops.push(Op::read(&mut reader)?);
	}

	if !reader.bytes.is_empty() {
		return Err(DecodeError("bytes are left over"));
	}

	Ok(ops)
}

/// Encodes a batch of events for [`decode_events`].
pub fn encode_events(events: &[Event]) -> Vec<u8> {
	let mut writer = Writer(Vec::with_capacity(9 + events.len() * 32));
	writer.0.extend_from_slice(EVENTS_MAGIC);
	writer.u8(VERSION);
	writer.u32(events.len() as u32);

	for event in events {
		event.write(&mut writer);
	}

	writer.0
}

/// Decodes a batch written by [`encode_events`].
pub fn decode_events(bytes: &[u8]) -> Result<Vec<Event>, DecodeError> {
	let mut reader = Reader { bytes };
	let count = reader.header(*EVENTS_MAGIC)?;

	if count > reader.bytes.len() / 5 + 1 {
		return Err(DecodeError("it ends early"));
	}

	let mut events = Vec::with_capacity(count);

	for _ in 0..count {
		events.push(Event::read(&mut reader)?);
	}

	if !reader.bytes.is_empty() {
		return Err(DecodeError("bytes are left over"));
	}

	Ok(events)
}
