//! Hit testing against the scene in its current pose, on the CPU.
//!
//! A ray first tests each part's bounds, then the triangles of the parts it
//! meets. Skinned and morphed parts are posed vertex by vertex on demand, so
//! a ray costs nothing for parts it misses.

use glam::Mat4;
use glam::Vec3;

use crate::asset::Aabb;
use crate::asset::Asset;
use crate::asset::Primitive;
use crate::protocol::Hit;
use crate::protocol::NodeId;
use crate::sanitize::MAX_HITS;
use crate::world::Instance;
use crate::world::World;

/// Where a ray enters a box, if it does within `max`.
fn ray_box(origin: Vec3, inverse: Vec3, bounds: &Aabb, max: f32) -> Option<f32> {
	if bounds.is_empty() {
		return None;
	}

	let a = (bounds.min - origin) * inverse;
	let b = (bounds.max - origin) * inverse;
	let near = a.min(b).max_element().max(0.0);
	let far = a.max(b).min_element().min(max);
	(near <= far).then_some(near)
}

/// Möller–Trumbore, both faces.
#[allow(clippy::many_single_char_names)]
fn ray_triangle(origin: Vec3, direction: Vec3, [a, b, c]: [Vec3; 3]) -> Option<f32> {
	let edge1 = b - a;
	let edge2 = c - a;
	let p = direction.cross(edge2);
	let determinant = edge1.dot(p);

	if determinant.abs() < 1.0e-12 {
		return None;
	}

	let inverse = 1.0 / determinant;
	let t_vector = origin - a;

	let u = t_vector.dot(p) * inverse;

	if !(0.0..=1.0).contains(&u) {
		return None;
	}

	let q = t_vector.cross(edge1);
	let v = direction.dot(q) * inverse;

	if v < 0.0 || u + v > 1.0 {
		return None;
	}

	let t = edge2.dot(q) * inverse;
	(t >= 0.0).then_some(t)
}

/// Poses one vertex of a primitive: morph targets, then skin or the part's
/// matrix.
struct Poser<'a> {
	asset: &'a Asset,
	primitive: &'a Primitive,
	model: Mat4,
	joints: Option<Vec<Mat4>>,
	weights: &'a [f32],
}

impl Poser<'_> {
	fn position(&self, index: u32) -> Vec3 {
		let vertex = (self.primitive.base_vertex + index) as usize;
		let mut position = Vec3::from(self.asset.vertices[vertex].position);

		if let Some(morph) = self.primitive.morph {
			for (target, weight) in self.weights.iter().enumerate().take(morph.targets as usize) {
				if *weight != 0.0 {
					let at = morph.base as usize
						+ (target * self.primitive.vertex_count as usize + index as usize) * 2;
					let delta = self.asset.morph_deltas[at];
					position += Vec3::new(delta[0], delta[1], delta[2]) * *weight;
				}
			}
		}

		match (&self.joints, self.primitive.skin_base) {
			(Some(joints), Some(base)) => {
				let influence = self.asset.skin_vertices[(base + index) as usize];
				let mut skinned = Vec3::ZERO;
				for slot in 0..4 {
					let weight = f32::from(influence.weights[slot]) / 65535.0;

					if weight > 0.0
						&& let Some(joint) = joints.get(usize::from(influence.joints[slot]))
					{
						skinned += joint.transform_point3(position) * weight;
					}
				}

				skinned
			}
			_ => self.model.transform_point3(position),
		}
	}
}

/// The skin's joint matrices in world space, or `None` for an unskinned part.
pub(crate) fn joint_matrices(asset: &Asset, instance: &Instance, node: usize) -> Option<Vec<Mat4>> {
	let skin = &asset.skins[asset.nodes[node].skin?];
	Some(
		skin.joints
			.iter()
			.zip(&skin.inverse_bind)
			.map(|(joint, inverse)| instance.world[*joint] * *inverse)
			.collect(),
	)
}

/// The bounds of a skinned part in its current pose, from its joints'
/// reach.
pub(crate) fn skinned_bounds(asset: &Asset, instance: &Instance, node: usize) -> Option<Aabb> {
	let skin = &asset.skins[asset.nodes[node].skin?];
	let mut bounds = Aabb::EMPTY;

	for (joint, radius) in skin.joints.iter().zip(&skin.joint_radius) {
		let world = instance.world[*joint];
		let scale = world
			.x_axis
			.truncate()
			.length()
			.max(world.y_axis.truncate().length())
			.max(world.z_axis.truncate().length());
		let center = world.w_axis.truncate();
		let reach = Vec3::splat(radius * scale);
		bounds.grow(center - reach);

		bounds.grow(center + reach);
	}

	Some(bounds)
}

fn instance_hits(
	id: NodeId,
	instance: &Instance,
	origin: Vec3,
	direction: Vec3,
	max_distance: f32,
	hits: &mut Vec<Hit>,
) {
	let (Some(asset), Some(pose)) = (&instance.asset, &instance.pose) else {
		return;
	};
	let inverse = direction.recip();

	for (index, node) in asset.nodes.iter().enumerate() {
		let Some(mesh) = node.mesh else {
			continue;
		};

		if instance.hidden[index] || instance.overrides[index].opacity <= 0.0 {
			continue;
		}

		let joints = joint_matrices(asset, instance, index);
		let model = instance.world[index];
		let skinned = skinned_bounds(asset, instance, index);
		let mut nearest: Option<(f32, [Vec3; 3])> = None;

		for primitive in &asset.meshes[mesh].primitives {
			let bounds = match (skinned, primitive.skin_base) {
				(Some(bounds), Some(_)) => bounds,
				_ => primitive.bounds.transformed(&model),
			};

			let limit = nearest.map_or(max_distance, |(distance, _)| distance);

			if ray_box(origin, inverse, &bounds, limit).is_none() {
				continue;
			}

			let poser = Poser {
				asset,
				primitive,
				model,
				joints: joints.clone(),
				weights: &pose.weights[index],
			};
			let indices = &asset.indices[primitive.first_index as usize
				..(primitive.first_index + primitive.index_count) as usize];

			for triangle in indices.as_chunks::<3>().0 {
				let corners =
					[triangle[0], triangle[1], triangle[2]].map(|vertex| poser.position(vertex));

				if let Some(distance) = ray_triangle(origin, direction, corners)
					&& distance <= nearest.map_or(max_distance, |(nearest, _)| nearest)
				{
					nearest = Some((distance, corners));
				}
			}
		}

		if let Some((distance, [a, b, c])) = nearest {
			let mut normal = (b - a).cross(c - a).normalize_or_zero();

			if normal.dot(direction) > 0.0 {
				normal = -normal;
			}

			hits.push(Hit {
				node: id,
				part: node.name.clone(),
				distance,
				point: (origin + direction * distance).to_array(),
				normal: normal.to_array(),
			});
		}
	}
}

/// Casts a ray through every visible instance. `direction` is unit length.
pub(crate) fn raycast(world: &World, origin: Vec3, direction: Vec3, max_distance: f32) -> Vec<Hit> {
	let mut hits = Vec::new();
	let mut ids: Vec<&NodeId> = world.nodes.keys().collect();
	ids.sort_unstable();

	for id in ids {
		let node = &world.nodes[id];

		if node.hidden {
			continue;
		}

		if let Some(instance) = &node.instance {
			instance_hits(*id, instance, origin, direction, max_distance, &mut hits);
		}
	}

	hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
	hits.truncate(MAX_HITS);
	hits
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rays_meet_triangles_from_either_side() {
		let triangle = [
			Vec3::new(-1.0, -1.0, 0.0),
			Vec3::new(1.0, -1.0, 0.0),
			Vec3::new(0.0, 1.0, 0.0),
		];
		assert_eq!(
			ray_triangle(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z, triangle),
			Some(5.0)
		);
		assert_eq!(
			ray_triangle(Vec3::new(0.0, 0.0, -2.0), Vec3::Z, triangle),
			Some(2.0)
		);
		assert_eq!(
			ray_triangle(Vec3::new(3.0, 0.0, 5.0), Vec3::NEG_Z, triangle),
			None
		);
		assert_eq!(
			ray_triangle(Vec3::new(0.0, 0.0, 5.0), Vec3::Z, triangle),
			None
		);
		assert_eq!(
			ray_triangle(Vec3::new(0.0, 0.0, 5.0), Vec3::X, triangle),
			None
		);
	}

	#[test]
	fn rays_enter_boxes_within_range() {
		let bounds = Aabb {
			min: Vec3::splat(-1.0),
			max: Vec3::splat(1.0),
		};
		let origin = Vec3::new(0.0, 0.0, 5.0);
		let inverse = Vec3::NEG_Z.recip();
		assert_eq!(ray_box(origin, inverse, &bounds, 10.0), Some(4.0));
		assert_eq!(ray_box(origin, inverse, &bounds, 3.0), None);
		assert_eq!(ray_box(Vec3::ZERO, inverse, &bounds, 3.0), Some(0.0));
		assert_eq!(ray_box(origin, inverse, &Aabb::EMPTY, 10.0), None);
	}
}
