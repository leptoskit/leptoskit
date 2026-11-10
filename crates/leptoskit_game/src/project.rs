//! Projects cubes into screen-space quads. The simulation never reads this.

use glam::Mat4;
use glam::Vec3;
use glam::Vec4;

use crate::world::Mesh;
use crate::world::Transform;
use crate::world::World;

/// The camera that frames the world. It does not affect the simulation.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
	/// The eye, in world units.
	pub eye: Vec3,
	/// The point the eye looks at.
	pub target: Vec3,
	/// World up.
	pub up: Vec3,
	/// Vertical field of view, in degrees.
	pub fov_y_degrees: f32,
}

impl Default for Camera {
	fn default() -> Self {
		Self {
			eye: Vec3::new(2.4, 1.8, 3.6),
			target: Vec3::new(0.0, 0.4, 0.0),
			up: Vec3::Y,
			fov_y_degrees: 50.0,
		}
	}
}

/// One filled convex quad, already in pixels. `depth` is view-space z; larger
/// (less negative) is farther.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Quad {
	pub points: [(f32, f32); 4],
	pub depth: f32,
	pub color: [u8; 4],
}

const CORNERS: [[f32; 3]; 8] = [
	[-0.5, -0.5, -0.5],
	[0.5, -0.5, -0.5],
	[0.5, 0.5, -0.5],
	[-0.5, 0.5, -0.5],
	[-0.5, -0.5, 0.5],
	[0.5, -0.5, 0.5],
	[0.5, 0.5, 0.5],
	[-0.5, 0.5, 0.5],
];

/// Outward faces, counter-clockwise when seen from outside.
const FACES: [[usize; 4]; 6] = [
	[1, 5, 6, 2],
	[4, 0, 3, 7],
	[3, 2, 6, 7],
	[0, 4, 5, 1],
	[4, 5, 6, 7],
	[1, 0, 3, 2],
];

/// The sky behind the world.
pub(crate) const SKY: [u8; 4] = [20, 26, 34, 255];

pub(crate) fn quads(world: &World, camera: &Camera, width: u32, height: u32) -> Vec<Quad> {
	let aspect = width.max(1) as f32 / height.max(1) as f32;
	let view = glam::camera::rh::view::look_at_mat4(camera.eye, camera.target, camera.up);
	let projection = glam::camera::rh::proj::opengl::perspective(
		camera.fov_y_degrees.to_radians(),
		aspect,
		0.05,
		100.0,
	);
	let clip_from_world = projection * view;
	let framed = Projection {
		eye: camera.eye,
		view,
		clip: clip_from_world,
		width,
		height,
	};
	let mut quads = Vec::new();
	for (transform, mesh) in world.bodies() {
		push_body(&mut quads, transform, mesh, &framed);
	}
	quads.sort_by(|left, right| right.depth.total_cmp(&left.depth));
	quads
}

struct Projection {
	eye: Vec3,
	view: Mat4,
	clip: Mat4,
	width: u32,
	height: u32,
}

fn push_body(quads: &mut Vec<Quad>, transform: &Transform, mesh: &Mesh, projection: &Projection) {
	let world_of = |corner: [f32; 3]| {
		Vec3::new(
			transform.position[0] + corner[0] * mesh.scale[0],
			transform.position[1] + corner[1] * mesh.scale[1],
			transform.position[2] + corner[2] * mesh.scale[2],
		)
	};
	for face in FACES {
		let corners = face.map(|index| world_of(CORNERS[index]));
		let normal = (corners[1] - corners[0]).cross(corners[3] - corners[0]);
		if normal.length_squared() < 1e-8 || normal.dot(projection.eye - corners[0]) <= 0.0 {
			continue;
		}
		let mut points = [(0.0, 0.0); 4];
		let mut depth = 0.0;
		let mut visible = true;
		for (slot, corner) in points.iter_mut().zip(corners) {
			let Some(projected) = project(corner, projection) else {
				visible = false;
				break;
			};
			*slot = (projected.0, projected.1);
			depth += projected.2;
		}
		if !visible {
			continue;
		}
		quads.push(Quad {
			points,
			depth: depth / 4.0,
			color: shade(mesh.color, normal),
		});
	}
}

fn project(point: Vec3, projection: &Projection) -> Option<(f32, f32, f32)> {
	let clip = projection.clip * Vec4::new(point.x, point.y, point.z, 1.0);
	if clip.w <= 1e-4 {
		return None;
	}
	let ndc = clip.truncate() / clip.w;
	if !ndc.is_finite() || ndc.z < -1.0 || ndc.z > 1.0 {
		return None;
	}
	let x = f32::midpoint(ndc.x, 1.0) * projection.width as f32;
	let y = f32::midpoint(-ndc.y, 1.0) * projection.height as f32;
	let view_z = (projection.view * point.extend(1.0)).z;
	Some((x, y, view_z))
}

fn shade(color: [u8; 4], normal: Vec3) -> [u8; 4] {
	let light = Vec3::new(0.35, 0.85, 0.25).normalize();
	let lambert = normal.normalize().dot(light).max(0.0);
	let shade = 0.35 + 0.65 * lambert;
	[
		(f32::from(color[0]) * shade) as u8,
		(f32::from(color[1]) * shade) as u8,
		(f32::from(color[2]) * shade) as u8,
		color[3],
	]
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_cube_in_front_of_the_camera_has_visible_faces() {
		let mut world = World::new();
		let _ = world.cube("box").at(0.0, 0.5, 0.0).color("#e7b15a");
		let faces = quads(&world, &Camera::default(), 320, 200);
		assert!(!faces.is_empty() && faces.len() <= 3, "{}", faces.len());
		assert!(faces.windows(2).all(|pair| pair[0].depth >= pair[1].depth));
	}

	#[test]
	fn a_cube_behind_the_camera_is_dropped() {
		let mut world = World::new();
		let _ = world.cube("gone").at(0.0, 0.0, 20.0);
		assert!(quads(&world, &Camera::default(), 64, 64).is_empty());
	}
}
