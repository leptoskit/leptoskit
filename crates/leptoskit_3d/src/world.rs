//! The retained scene: the guest's nodes, the model instances among them,
//! their animation, the camera, and the light. It holds no GPU objects, so
//! it can be driven and tested without a device.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;

use glam::Mat4;
use glam::Quat;
use glam::Vec3;

use crate::animation::Animator;
use crate::animation::Pose;
use crate::asset::Asset;
use crate::protocol::*;
use crate::sanitize::*;

/// A part's material changes, resolved from every `SetMaterial` that covers
/// it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Overrides {
	pub color: [f32; 4],
	pub emissive: Option<[f32; 3]>,
	pub emissive_strength: Option<f32>,
	pub opacity: f32,
	pub rim: Rim,
	pub blend: Blend,
}

impl Default for Overrides {
	fn default() -> Self {
		Self {
			color: [1.0; 4],
			emissive: None,
			emissive_strength: None,
			opacity: 1.0,
			rim: Rim {
				color: [1.0; 3],
				strength: 0.0,
				power: 2.0,
			},
			blend: Blend::Normal,
		}
	}
}

impl Overrides {
	/// Applies the fields an override sets, leaving the others as they were.
	fn apply(&mut self, change: &MaterialOverride) {
		if let Some(color) = change.color {
			self.color = color;
		}

		if change.emissive.is_some() {
			self.emissive = change.emissive;
		}

		if change.emissive_strength.is_some() {
			self.emissive_strength = change.emissive_strength;
		}

		if let Some(opacity) = change.opacity {
			self.opacity = opacity;
		}

		if let Some(rim) = change.rim {
			self.rim = rim;
		}
	}
}

fn merge(into: &mut MaterialOverride, change: &MaterialOverride) {
	into.color = change.color.or(into.color);
	into.emissive = change.emissive.or(into.emissive);
	into.emissive_strength = change.emissive_strength.or(into.emissive_strength);
	into.opacity = change.opacity.or(into.opacity);
	into.rim = change.rim.or(into.rim);
}

/// A part's offset, as the pose composes it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Motion {
	translation: Vec3,
	rotation: Quat,
	scale: Vec3,
}

impl Motion {
	const IDENTITY: Self = Self {
		translation: Vec3::ZERO,
		rotation: Quat::IDENTITY,
		scale: Vec3::ONE,
	};

	fn from_transform(transform: &Transform) -> Self {
		Self {
			translation: Vec3::from(transform.translation),
			rotation: Quat::from_array(transform.rotation),
			scale: Vec3::from(transform.scale),
		}
	}

	/// The motion `t` of the way from `self` to `to`, along a straight line
	/// and the shorter arc.
	fn towards(self, to: Self, t: f32) -> Self {
		Self {
			translation: self.translation.lerp(to.translation, t),
			rotation: self.rotation.slerp(to.rotation, t),
			scale: self.scale.lerp(to.scale, t),
		}
	}
}

/// A part the guest has moved, and how far it is on its way.
#[derive(Clone, Copy, Debug)]
struct PartOffset {
	/// The asset node it moves, once the asset has loaded, when it has the
	/// part.
	node: Option<usize>,
	from: Motion,
	to: Motion,
	seconds: f32,
	elapsed: f32,
}

impl PartOffset {
	/// Where the part is on its way now.
	fn current(&self) -> Motion {
		if self.elapsed >= self.seconds {
			self.to
		} else {
			self.from.towards(self.to, self.elapsed / self.seconds)
		}
	}

	fn is_moving(&self) -> bool {
		self.elapsed < self.seconds
	}
}

/// The most material, visibility, or morph settings one instance keeps.
const MAX_SETTINGS: usize = 1024;

/// A model in the scene: what the guest asked of it, and once its asset has
/// loaded, its pose.
pub(crate) struct Instance {
	pub asset_id: AssetId,
	pub asset: Option<Arc<Asset>>,
	pub cast_shadow: bool,
	parts_visible: BTreeMap<String, bool>,
	materials: Vec<(Option<String>, MaterialOverride)>,
	blends: Vec<(Option<String>, Blend)>,
	morphs: BTreeMap<(Option<String>, String), f32>,
	offsets: BTreeMap<String, PartOffset>,
	pending_play: Option<Play>,
	animator: Animator,
	rest: Option<Pose>,
	pub pose: Option<Pose>,
	/// Per asset node, once loaded.
	pub world: Vec<Mat4>,
	pub hidden: Vec<bool>,
	pub overrides: Vec<Overrides>,
	manual_weights: Vec<Vec<Option<f32>>>,
	touched: Vec<usize>,
}

impl Instance {
	fn new(asset_id: AssetId) -> Self {
		Self {
			asset_id,
			asset: None,
			cast_shadow: true,
			parts_visible: BTreeMap::new(),
			materials: Vec::new(),
			blends: Vec::new(),
			morphs: BTreeMap::new(),
			offsets: BTreeMap::new(),
			pending_play: None,
			animator: Animator::default(),
			rest: None,
			pose: None,
			world: Vec::new(),
			hidden: Vec::new(),
			overrides: Vec::new(),
			manual_weights: Vec::new(),
			touched: Vec::new(),
		}
	}

	fn bind(&mut self, asset: Arc<Asset>) {
		let rest = Pose::rest(&asset);
		self.pose = Some(rest.clone());
		self.rest = Some(rest);
		self.world = vec![Mat4::IDENTITY; asset.nodes.len()];
		self.asset = Some(asset);
		self.resolve_visibility();
		self.resolve_materials();
		self.resolve_morphs();
		self.resolve_offsets();

		if let Some(play) = self.pending_play.take() {
			self.play(&play);
		}
	}

	/// The asset nodes under `part`, itself included, or every node when
	/// `part` is `None`. Empty when the part does not exist.
	fn subtree(asset: &Asset, part: Option<&str>) -> Vec<usize> {
		let starts: Vec<usize> = match part {
			None => asset.roots.clone(),
			Some(name) => asset.node_named(name).into_iter().collect(),
		};

		let mut nodes = Vec::new();
		let mut stack = starts;

		while let Some(node) = stack.pop() {
			nodes.push(node);
			stack.extend(asset.nodes[node].children.iter().copied());
		}

		nodes
	}

	fn resolve_visibility(&mut self) {
		let Some(asset) = &self.asset else {
			return;
		};
		self.hidden = vec![false; asset.nodes.len()];

		for (part, visible) in &self.parts_visible {
			if *visible {
				continue;
			}

			for node in Self::subtree(asset, Some(part)) {
				self.hidden[node] = true;
			}
		}
	}

	fn resolve_materials(&mut self) {
		let Some(asset) = &self.asset else {
			return;
		};
		self.overrides = vec![Overrides::default(); asset.nodes.len()];

		for (part, change) in &self.materials {
			for node in Self::subtree(asset, part.as_deref()) {
				self.overrides[node].apply(change);
			}
		}

		for (part, blend) in &self.blends {
			for node in Self::subtree(asset, part.as_deref()) {
				self.overrides[node].blend = *blend;
			}
		}
	}

	/// Replaces the blend set for `part`, or adds it while there is room. A
	/// later setting wins where parts overlap.
	fn set_blend(&mut self, part: Option<String>, blend: Blend) {
		self.blends.retain(|(existing, _)| *existing != part);

		if self.blends.len() < MAX_SETTINGS {
			self.blends.push((part, blend));
		}

		self.resolve_materials();
	}

	/// Applies morph weights set by name before the asset loaded, now that
	/// its meshes say which target each name is.
	fn resolve_morphs(&mut self) {
		let Some(asset) = &self.asset else {
			return;
		};
		self.manual_weights = asset
			.nodes
			.iter()
			.map(|node| {
				node.mesh
					.map(|mesh| vec![None; asset.meshes[mesh].weights.len()])
					.unwrap_or_default()
			})
			.collect();

		for ((part, target), weight) in &self.morphs {
			for node in Self::subtree(asset, part.as_deref()) {
				for index in target_indices(asset, node, target) {
					self.manual_weights[node][index] = Some(*weight);
				}
			}
		}
	}

	/// Finds the node each offset moves, now that the asset says which
	/// node each name is.
	fn resolve_offsets(&mut self) {
		let Some(asset) = &self.asset else {
			return;
		};

		for (part, offset) in &mut self.offsets {
			offset.node = asset.node_named(part);
		}
	}

	/// Starts `part` on its way to `offset` from where it is now, or puts it
	/// there at once.
	fn set_part_offset(&mut self, part: String, offset: &Transform, seconds: f32) {
		let to = Motion::from_transform(offset);
		let from = self
			.offsets
			.get(&part)
			.map_or(Motion::IDENTITY, PartOffset::current);

		if seconds <= 0.0 && to == Motion::IDENTITY {
			self.offsets.remove(&part);

			return;
		}

		if self.offsets.len() >= MAX_SETTINGS && !self.offsets.contains_key(&part) {
			return;
		}

		let node = self
			.asset
			.as_ref()
			.and_then(|asset| asset.node_named(&part));
		self.offsets.insert(
			part,
			PartOffset {
				node,
				from,
				to,
				seconds: seconds.max(0.0),
				elapsed: 0.0,
			},
		);
	}

	/// Composes every offset onto the pose, in each part's own frame, and
	/// forgets the ones that have come back to rest.
	fn apply_offsets(&mut self, elapsed: f32) {
		let Some(pose) = &mut self.pose else {
			return;
		};

		for offset in self.offsets.values_mut() {
			offset.elapsed = (offset.elapsed + elapsed).min(offset.seconds);
			let Some(node) = offset.node else {
				continue;
			};
			let motion = offset.current();
			let (rotation, scale) = (pose.rotation[node], pose.scale[node]);
			pose.translation[node] += rotation * (scale * motion.translation);
			pose.rotation[node] = (rotation * motion.rotation).normalize();
			pose.scale[node] = scale * motion.scale;
		}

		self.offsets
			.retain(|_, offset| offset.is_moving() || offset.to != Motion::IDENTITY);
	}

	fn play(&mut self, play: &Play) {
		let Some(asset) = &self.asset else {
			self.pending_play = Some(play.clone());

			return;
		};

		if let Some(index) = asset.clips.iter().position(|clip| clip.name == play.clip) {
			let duration = asset.clips[index].duration;
			self.animator
				.play(index, play.speed, play.mode, play.fade, duration);
		}
	}

	fn set_part_visible(&mut self, part: String, visible: bool) {
		if self.parts_visible.len() < MAX_SETTINGS || self.parts_visible.contains_key(&part) {
			self.parts_visible.insert(part, visible);
			self.resolve_visibility();
		}
	}

	fn stop(&mut self, fade: f32) {
		self.pending_play = None;
		self.animator.stop(fade);
	}

	fn set_morph(&mut self, part: Option<String>, target: String, weight: f32) {
		let key = (part, target);

		if self.morphs.len() < MAX_SETTINGS || self.morphs.contains_key(&key) {
			self.morphs.insert(key, weight);
			self.resolve_morphs();
		}
	}

	/// Merges a material change into the one already set for `part`, or adds
	/// it while there is room.
	fn set_material(&mut self, part: Option<String>, material: &MaterialOverride) {
		let full = self.materials.len() >= MAX_SETTINGS;
		let existing = self
			.materials
			.iter_mut()
			.find(|(existing, _)| *existing == part);

		match existing {
			Some((_, existing)) => merge(existing, material),

			None if !full => self.materials.push((part, *material)),
			None => return,
		}

		self.resolve_materials();
	}

	/// Advances the animation and samples the pose. Returns the clips played
	/// once that finished.
	fn advance(&mut self, elapsed: f32) -> Vec<String> {
		let (Some(asset), Some(rest), Some(pose)) = (&self.asset, &self.rest, &mut self.pose)
		else {
			return Vec::new();
		};
		let advance = self.animator.advance(asset, elapsed);

		if self.animator.is_active() {
			self.animator.sample(asset, rest, pose, &mut self.touched);
		} else {
			pose.translation.copy_from_slice(&rest.translation);

			pose.rotation.copy_from_slice(&rest.rotation);
			pose.scale.copy_from_slice(&rest.scale);

			for (weights, rest) in pose.weights.iter_mut().zip(&rest.weights) {
				weights.copy_from_slice(rest);
			}
		}

		for (weights, manual) in pose.weights.iter_mut().zip(&self.manual_weights) {
			for (weight, manual) in weights.iter_mut().zip(manual) {
				if let Some(manual) = manual {
					*weight = *manual;
				}
			}
		}

		let finished = advance
			.finished
			.into_iter()
			.map(|clip| asset.clips[clip].name.clone())
			.collect();
		self.apply_offsets(elapsed);

		finished
	}

	/// Computes every asset node's world matrix under `root`.
	fn update_world(&mut self, root: Mat4) {
		let (Some(asset), Some(pose)) = (&self.asset, &self.pose) else {
			return;
		};
		let mut stack: Vec<(usize, Mat4)> = asset.roots.iter().map(|node| (*node, root)).collect();

		while let Some((node, parent)) = stack.pop() {
			let local = Mat4::from_scale_rotation_translation(
				pose.scale[node],
				pose.rotation[node],
				pose.translation[node],
			);
			let world = parent * local;
			self.world[node] = world;

			for child in &asset.nodes[node].children {
				stack.push((*child, world));
			}
		}
	}

	/// Whether this instance would change if time moved.
	fn is_moving(&self) -> bool {
		self.animator.is_moving() || self.offsets.values().any(PartOffset::is_moving)
	}
}

/// The indices of the morph targets named `target` on `node`'s mesh.
fn target_indices<'a>(
	asset: &'a Asset,
	node: usize,
	target: &'a str,
) -> impl Iterator<Item = usize> + 'a {
	asset.nodes[node]
		.mesh
		.into_iter()
		.flat_map(|mesh| asset.meshes[mesh].target_names.iter().enumerate())
		.filter(move |(_, name)| *name == target)
		.map(|(index, _)| index)
}

/// A node of the guest's scene.
pub(crate) struct Node {
	pub parent: Option<NodeId>,
	pub children: Vec<NodeId>,
	/// The part of the parent instance this node follows, after an attach.
	pub bone: Option<String>,
	pub transform: Transform,
	pub visible: bool,
	pub instance: Option<Instance>,
	pub world: Mat4,
	/// Hidden itself or under a hidden node.
	pub hidden: bool,
}

/// Where the camera is and how it projects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Camera {
	Perspective {
		transform: Transform,
		fov_y: f32,
		near: f32,
		far: f32,
	},
	Matrices {
		view: Mat4,
		projection: Mat4,
	},
}

impl Default for Camera {
	fn default() -> Self {
		Self::Perspective {
			transform: Transform::from_translation(0.0, 1.0, 4.0),
			fov_y: 0.8,
			near: 0.05,
			far: 200.0,
		}
	}
}

impl Camera {
	/// The view and projection for a viewport of `aspect` (width over
	/// height), depth from 0 to 1.
	pub fn matrices(&self, aspect: f32) -> (Mat4, Mat4) {
		match *self {
			Self::Perspective {
				transform,
				fov_y,
				near,
				far,
			} => {
				let world = Mat4::from_scale_rotation_translation(
					Vec3::ONE,
					Quat::from_array(transform.rotation),
					Vec3::from(transform.translation),
				);
				(
					world.inverse(),
					glam::camera::rh::proj::directx::perspective(
						fov_y,
						aspect.max(1.0e-3),
						near,
						far,
					),
				)
			}
			Self::Matrices { view, projection } => (view, projection),
		}
	}
}

/// Scene-wide settings: the light, what draws behind, and exposure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Settings {
	pub light: Light,
	pub backdrop: Backdrop,
	pub exposure: f32,
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			light: Light::default(),
			backdrop: Backdrop::Transparent,
			exposure: 1.0,
		}
	}
}

/// How the scene's assets stand.
enum Slot {
	Loading,
	Ready(Arc<Asset>),
	Failed,
}

/// A scene: what a guest's ops build, and what a renderer draws.
///
/// Feed it ops with [`World::apply`], move time forward with
/// [`World::advance`], and hand it the assets it asks for with
/// [`World::take_loads`] and [`World::asset_loaded`].
#[derive(Default)]
pub struct World {
	pub(crate) nodes: HashMap<NodeId, Node>,
	pub(crate) roots: Vec<NodeId>,
	assets: HashMap<AssetId, Slot>,
	loads: Vec<(AssetId, String)>,
	events: VecDeque<Event>,
	pub(crate) camera: Camera,
	pub(crate) settings: Settings,
	aspect: f32,
	raycasts: usize,
	changed: bool,
	dropped_events: u64,
}

impl World {
	/// An empty scene with the default camera and light.
	pub fn new() -> Self {
		Self {
			aspect: 1.0,
			changed: true,
			..Self::default()
		}
	}

	/// Applies a batch of ops, sanitizing each first.
	pub fn apply(&mut self, ops: impl IntoIterator<Item = Op>) {
		self.raycasts = 0;

		for op in ops {
			// A raycast reads the scene without changing what it shows.
			self.changed |= !matches!(op, Op::Raycast { .. } | Op::RaycastScreen { .. });
			self.apply_one(op.sanitized());
		}
	}

	/// Asset loads the scene is waiting for, as `(id, path)`; the host reads
	/// each and answers with [`World::asset_loaded`].
	pub fn take_loads(&mut self) -> Vec<(AssetId, String)> {
		std::mem::take(&mut self.loads)
	}

	/// Hands the scene an asset it asked for, or why it failed.
	pub fn asset_loaded(&mut self, id: AssetId, result: Result<Arc<Asset>, String>) {
		if !matches!(self.assets.get(&id), Some(Slot::Loading)) {
			return;
		}

		match result {
			Ok(asset) => {
				let clips = asset.clip_names().map(str::to_owned).collect();
				let waiting = self
					.nodes
					.values_mut()
					.filter_map(|node| node.instance.as_mut())
					.filter(|instance| instance.asset_id == id && instance.asset.is_none());

				for instance in waiting {
					instance.bind(asset.clone());
				}

				self.assets.insert(id, Slot::Ready(asset));
				self.push_event(Event::AssetLoaded { asset: id, clips });
			}
			Err(message) => {
				self.assets.insert(id, Slot::Failed);
				self.push_event(Event::AssetFailed { asset: id, message });
			}
		}

		self.changed = true;
	}

	/// Takes the events waiting for the guest, oldest first.
	pub fn take_events(&mut self) -> Vec<Event> {
		self.events.drain(..).collect()
	}

	/// Puts events back at the front of the queue, in order, for a guest
	/// that could not take them all at once.
	#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
	pub(crate) fn requeue_events(&mut self, events: Vec<Event>) {
		for event in events.into_iter().rev() {
			self.events.push_front(event);
		}
	}

	/// Whether events are waiting.
	pub fn has_events(&self) -> bool {
		!self.events.is_empty()
	}

	/// What the scene asked to draw behind it.
	pub fn backdrop(&self) -> Backdrop {
		self.settings.backdrop
	}

	/// How many events were dropped because the guest did not take them.
	pub fn dropped_events(&self) -> u64 {
		self.dropped_events
	}

	/// Whether the scene needs drawing again: it changed since the last
	/// [`World::advance`], or something in it is animating.
	pub fn needs_frame(&self) -> bool {
		self.changed
			|| self
				.nodes
				.values()
				.any(|node| node.instance.as_ref().is_some_and(Instance::is_moving))
	}

	/// How many nodes the scene has.
	pub fn node_count(&self) -> usize {
		self.nodes.len()
	}

	/// The aspect ratio (width over height) the camera projects for, which
	/// screen raycasts use. The renderer sets it from the view it draws.
	pub fn set_aspect(&mut self, aspect: f32) {
		if aspect.is_finite() && aspect > 0.0 {
			self.aspect = aspect;
		}
	}

	fn push_event(&mut self, event: Event) {
		if self.events.len() >= MAX_PENDING_EVENTS {
			self.events.pop_front();
			self.dropped_events += 1;
		}

		self.events.push_back(event);
	}

	fn instance_mut(&mut self, node: NodeId) -> Option<&mut Instance> {
		self.nodes.get_mut(&node)?.instance.as_mut()
	}

	/// Whether `node` is `ancestor` or under it.
	fn is_within(&self, node: NodeId, ancestor: NodeId) -> bool {
		let mut current = Some(node);

		while let Some(id) = current {
			if id == ancestor {
				return true;
			}

			current = self.nodes.get(&id).and_then(|node| node.parent);
		}

		false
	}

	fn detach(&mut self, id: NodeId) {
		let parent = self.nodes.get(&id).and_then(|node| node.parent);

		match parent {
			Some(parent) => {
				if let Some(parent) = self.nodes.get_mut(&parent) {
					parent.children.retain(|child| *child != id);
				}
			}
			None => self.roots.retain(|root| *root != id),
		}
	}

	fn adopt(&mut self, id: NodeId, parent: Option<NodeId>, bone: Option<String>) {
		if let Some(node) = self.nodes.get_mut(&id) {
			node.parent = parent;
			node.bone = bone;
		}

		match parent.and_then(|parent| self.nodes.get_mut(&parent)) {
			Some(parent) => parent.children.push(id),
			None => self.roots.push(id),
		}
	}

	/// Queues an asset to load, or reports at once why it will not: a path
	/// outside the assets, or a scene that already refers to its most.
	fn load(&mut self, asset: AssetId, path: String) {
		if self.assets.contains_key(&asset) {
			return;
		}

		let refusal = if !is_safe_asset_path(&path) {
			Some(format!(
				"{path:?} is not a relative path inside the app's assets"
			))
		} else if self.assets.len() >= MAX_ASSETS {
			Some("the scene already refers to the most assets it may".to_owned())
		} else {
			None
		};

		if let Some(message) = refusal {
			self.push_event(Event::AssetFailed { asset, message });

			return;
		}

		self.assets.insert(asset, Slot::Loading);
		self.loads.push((asset, path));
	}

	fn spawn(&mut self, node: NodeId, asset: Option<AssetId>, parent: Option<NodeId>) {
		if self.nodes.contains_key(&node) || self.nodes.len() >= MAX_NODES {
			return;
		}

		let instance = asset.map(|id| {
			let mut instance = Instance::new(id);
			if let Some(Slot::Ready(asset)) = self.assets.get(&id) {
				instance.bind(asset.clone());
			}

			instance
		});
		self.nodes.insert(
			node,
			Node {
				parent: None,
				children: Vec::new(),
				bone: None,
				transform: Transform::IDENTITY,
				visible: true,
				instance,
				world: Mat4::IDENTITY,
				hidden: false,
			},
		);
		let parent = parent.filter(|parent| self.nodes.contains_key(parent) && *parent != node);
		self.adopt(node, parent, None);
	}

	fn remove(&mut self, id: NodeId) {
		if !self.nodes.contains_key(&id) {
			return;
		}

		self.detach(id);
		let mut stack = vec![id];

		while let Some(id) = stack.pop() {
			if let Some(node) = self.nodes.remove(&id) {
				stack.extend(node.children);
			}
		}
	}

	/// Applies one sanitized op. Ops for nodes that are gone do nothing.
	#[allow(clippy::too_many_lines)]
	fn apply_one(&mut self, op: Op) {
		match op {
			Op::LoadAsset { asset, path } => self.load(asset, path),
			Op::UnloadAsset { asset } => {
				self.assets.remove(&asset);
				self.loads.retain(|(id, _)| *id != asset);
			}
			Op::Spawn {
				node,
				asset,
				parent,
			} => self.spawn(node, asset, parent),
			Op::Remove { node } => self.remove(node),
			Op::SetTransform { node, transform } => {
				if let Some(node) = self.nodes.get_mut(&node) {
					node.transform = transform;
				}
			}
			Op::SetVisible { node, visible } => {
				if let Some(node) = self.nodes.get_mut(&node) {
					node.visible = visible;
				}
			}
			Op::SetPartVisible {
				node,
				part,
				visible,
			} => self.with_instance(node, |instance| instance.set_part_visible(part, visible)),
			Op::Play { node, play } => self.with_instance(node, |instance| instance.play(&play)),
			Op::Stop { node, fade } => self.with_instance(node, |instance| instance.stop(fade)),
			Op::SetMorphWeight {
				node,
				part,
				target,
				weight,
			} => self.with_instance(node, |instance| instance.set_morph(part, target, weight)),
			Op::SetPartOffset {
				node,
				part,
				offset,
				seconds,
			} => {
				self.with_instance(node, |instance| {
					instance.set_part_offset(part, &offset, seconds);
				});
			}
			Op::SetMaterial {
				node,
				part,
				material,
			} => self.with_instance(node, |instance| instance.set_material(part, &material)),
			Op::SetBlend { node, part, blend } => {
				self.with_instance(node, |instance| instance.set_blend(part, blend));
			}
			Op::SetShadow { node, cast } => {
				self.with_instance(node, |instance| instance.cast_shadow = cast);
			}
			Op::Attach { node, target, bone } => self.attach(node, target, bone),
			Op::SetCamera {
				transform,
				fov_y,
				near,
				far,
			} => {
				self.camera = Camera::Perspective {
					transform,
					fov_y,
					near,
					far,
				};
			}
			Op::SetCameraMatrices { view, projection } => {
				self.camera = Camera::Matrices {
					view: Mat4::from_cols_array(&view),
					projection: Mat4::from_cols_array(&projection),
				};
			}
			Op::SetLight(light) => self.settings.light = light,
			Op::SetBackdrop(backdrop) => self.settings.backdrop = backdrop,
			Op::SetExposure(exposure) => self.settings.exposure = exposure,
			Op::Raycast {
				request,
				origin,
				direction,
				max_distance,
			} => {
				let hits =
					self.limited_raycast(Vec3::from(origin), Vec3::from(direction), max_distance);
				self.push_event(Event::RaycastHits { request, hits });
			}
			Op::RaycastScreen { request, x, y } => {
				let hits = self.raycast_screen(x, y);
				self.push_event(Event::RaycastHits { request, hits });
			}
		}
	}

	/// Runs `change` on the model instance at `node`, if there is one.
	fn with_instance(&mut self, node: NodeId, change: impl FnOnce(&mut Instance)) {
		if let Some(instance) = self.instance_mut(node) {
			change(instance);
		}
	}

	/// Moves `node` under `target`'s part `bone`, unless that would put a
	/// node under itself.
	fn attach(&mut self, node: NodeId, target: NodeId, bone: String) {
		if node == target
			|| !self.nodes.contains_key(&node)
			|| !self.nodes.contains_key(&target)
			|| self.is_within(target, node)
		{
			return;
		}

		self.detach(node);
		self.adopt(node, Some(target), (!bone.is_empty()).then_some(bone));
	}

	/// Casts a ray from the camera through a point of the view.
	fn raycast_screen(&mut self, x: f32, y: f32) -> Vec<Hit> {
		let (view, projection) = self.camera.matrices(self.aspect);
		let inverse = (projection * view).inverse();
		let ndc = Vec3::new(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0);
		let near = inverse.project_point3(ndc);
		let far = inverse.project_point3(ndc.with_z(1.0));

		match (far - near).try_normalize() {
			Some(direction) if near.is_finite() => {
				self.limited_raycast(near, direction, near.distance(far))
			}

			_ => Vec::new(),
		}
	}

	fn limited_raycast(&mut self, origin: Vec3, direction: Vec3, max_distance: f32) -> Vec<Hit> {
		self.raycasts += 1;

		if self.raycasts > MAX_RAYCASTS_PER_BATCH {
			return Vec::new();
		}

		// The ray needs the current pose, which the last advance computed;
		// bring transforms up to date for ops earlier in this batch.
		self.update_transforms();
		crate::raycast::raycast(self, origin, direction, max_distance)
	}

	/// Moves time forward by `elapsed` seconds: advances every clip and
	/// recomputes every transform.
	pub fn advance(&mut self, elapsed: f32) {
		let elapsed = if elapsed.is_finite() {
			elapsed.clamp(0.0, 0.25)
		} else {
			0.0
		};

		let mut finished = Vec::new();

		for (id, node) in &mut self.nodes {
			if let Some(instance) = &mut node.instance {
				for clip in instance.advance(elapsed) {
					finished.push(Event::AnimationFinished { node: *id, clip });
				}
			}
		}

		finished.sort_by_key(|event| {
			match event {
				Event::AnimationFinished { node, .. } => *node,
				_ => NodeId(0),
			}
		});

		for event in finished {
			self.push_event(event);
		}

		self.update_transforms();
		self.changed = false;
	}

	/// Recomputes every node's and part's world matrix from the current
	/// transforms and poses.
	pub(crate) fn update_transforms(&mut self) {
		let mut stack: Vec<(NodeId, Mat4, bool)> = self
			.roots
			.iter()
			.rev()
			.map(|root| (*root, Mat4::IDENTITY, false))
			.collect();

		while let Some((id, parent_world, parent_hidden)) = stack.pop() {
			let Some(node) = self.nodes.get(&id) else {
				continue;
			};
			let local = Mat4::from_scale_rotation_translation(
				Vec3::from(node.transform.scale),
				Quat::from_array(node.transform.rotation),
				Vec3::from(node.transform.translation),
			);
			let world = parent_world * local;
			let hidden = parent_hidden || !node.visible;
			let children = node.children.clone();

			let node = self.nodes.get_mut(&id).expect("the node exists");
			node.world = world;
			node.hidden = hidden;

			if let Some(instance) = &mut node.instance {
				instance.update_world(world);
			}

			for child in children.into_iter().rev() {
				let bone = self
					.nodes
					.get(&child)
					.and_then(|child| child.bone.as_deref());
				let base = bone
					.and_then(|bone| {
						let instance = self.nodes.get(&id)?.instance.as_ref()?;
						let asset = instance.asset.as_ref()?;
						let part = asset.node_named(bone)?;
						let hidden = instance.hidden.get(part).copied().unwrap_or(false);
						Some((instance.world[part], hidden))
					})
					.unwrap_or((world, false));
				stack.push((child, base.0, hidden || base.1));
			}
		}
	}
}
