//! Hit tests in software, against a frame's planes and depth picture.
//!
//! Backends with a native hit test (ARKit raycasts, ARCore, WebXR) use their
//! own for planes and fall back to these; the replay, orientation, and
//! headset backends use these alone.

use crate::image::DepthImage;
use crate::math::Pose;
use crate::math::Projection;
use crate::math::Quat;
use crate::math::Ray;
use crate::math::Vec3;
use crate::types::Hit;
use crate::types::HitKind;
use crate::types::HitOrigin;
use crate::types::HitTargets;
use crate::types::HitTestRequest;
use crate::types::Plane;
use crate::types::View;

/// How far a ray is followed through the depth picture, in metres.
const DEPTH_REACH: f32 = 10.0;
/// The step between depth samples along a ray, in metres.
const DEPTH_STEP: f32 = 0.02;
/// How far a neighbouring sample is taken to estimate a depth normal, as a
/// fraction of the view.
const NORMAL_SPAN: f32 = 0.01;

/// The world-space ray a request casts, or `None` for a screen point when
/// there is no view.
#[must_use]
pub fn request_ray(origin: HitOrigin, views: &[View]) -> Option<Ray> {
	match origin {
		HitOrigin::Ray(ray) => Some(ray),
		HitOrigin::Screen { x, y } => views.first()?.ray(x, y),
	}
}

/// Runs a request against `planes` and `depth`, nearest hit first.
#[must_use]
pub fn hit_test(
	request: &HitTestRequest,
	views: &[View],
	planes: &[Plane],
	depth: Option<&DepthImage>,
) -> Vec<Hit> {
	let Some(ray) = request_ray(request.origin, views) else {
		return Vec::new();
	};
	let mut hits = hit_planes(ray, planes, request.targets);

	if request.targets.contains(HitTargets::DEPTH)
		&& let (Some(depth), Some(view)) = (depth, views.first())
		&& let Some(hit) = hit_depth(ray, view, depth)
	{
		hits.push(hit);
	}

	sort_and_truncate(&mut hits, request.max_results as usize);
	hits
}

/// Orders hits nearest first and keeps at most `max`.
pub fn sort_and_truncate(hits: &mut Vec<Hit>, max: usize) {
	hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
	hits.truncate(max);
}

/// Hits against planes: inside their outlines for [`HitTargets::PLANES`],
/// anywhere on them for [`HitTargets::INFINITE_PLANES`].
#[must_use]
pub fn hit_planes(ray: Ray, planes: &[Plane], targets: HitTargets) -> Vec<Hit> {
	let bounded = targets.contains(HitTargets::PLANES);
	let infinite = targets.contains(HitTargets::INFINITE_PLANES);

	if !bounded && !infinite {
		return Vec::new();
	}

	planes
		.iter()
		.filter_map(|plane| {
			let (distance, local) = intersect_plane(ray, plane.pose)?;
			let inside = plane.contains_local(local.x, local.z);
			let kind = match (inside, bounded, infinite) {
				(true, true, _) => HitKind::Plane,
				(_, _, true) => HitKind::InfinitePlane,
				_ => return None,
			};

			Some(Hit {
				pose: Pose::new(ray.at(distance), plane.pose.orientation),
				distance,
				kind,
				plane: Some(plane.id),
			})
		})
		.collect()
}

/// Where a ray crosses the plane through `pose` with normal along its local
/// +Y, as the distance along the ray and the point in the plane's space. Rays
/// from behind the surface or parallel to it miss.
fn intersect_plane(ray: Ray, pose: Pose) -> Option<(f32, Vec3)> {
	let normal = pose.transform_vector(Vec3::Y);
	let facing = ray.direction.dot(normal);

	if facing > -1e-6 {
		return None;
	}

	let distance = (pose.position - ray.origin).dot(normal) / facing;

	(distance > 0.0).then(|| (distance, pose.inverse().transform_point(ray.at(distance))))
}

/// Where a point in the view's own space lands on the view, `(0, 0)` top-left
/// to `(1, 1)` bottom-right, or `None` behind it.
#[must_use]
pub fn project(projection: Projection, point: Vec3) -> Option<(f32, f32)> {
	if point.z >= -1e-6 {
		return None;
	}

	let (ndc_x, ndc_y) = match projection {
		Projection::Fov(fov) => {
			let (left, right) = (fov.left.tan(), fov.right.tan());
			let (up, down) = (fov.up.tan(), fov.down.tan());
			let (tan_x, tan_y) = (point.x / -point.z, point.y / -point.z);
			(
				(tan_x - left) / (right - left) * 2.0 - 1.0,
				(tan_y - down) / (up - down) * 2.0 - 1.0,
			)
		}
		Projection::Matrix(m) => {
			let w = -point.z;
			(
				(m[0] * point.x + m[8] * point.z) / w,
				(m[5] * point.y + m[9] * point.z) / w,
			)
		}
	};

	Some((f32::midpoint(ndc_x, 1.0), f32::midpoint(-ndc_y, 1.0)))
}

/// The world point the depth picture shows at a point on the view.
fn unproject(view: &View, depth: &DepthImage, x: f32, y: f32) -> Option<Vec3> {
	let meters = depth.sample_view(x, y)?;
	let direction = view.projection.direction(x, y);

	Some(view.pose.transform_point(direction * meters))
}

/// Follows a ray through the depth picture and returns where it first passes
/// behind a real surface, with a normal estimated from neighbouring samples.
#[must_use]
pub fn hit_depth(ray: Ray, view: &View, depth: &DepthImage) -> Option<Hit> {
	let to_view = view.pose.inverse();
	let steps = (DEPTH_REACH / DEPTH_STEP) as u32;

	(1..=steps).find_map(|step| {
		let distance = step as f32 * DEPTH_STEP;
		let local = to_view.transform_point(ray.at(distance));
		let (x, y) = project(view.projection, local)?;
		let surface = depth.sample_view(x, y)?;

		(-local.z >= surface).then(|| depth_hit(ray, view, depth, (x, y), distance))
	})
}

fn depth_hit(ray: Ray, view: &View, depth: &DepthImage, (x, y): (f32, f32), distance: f32) -> Hit {
	let point = unproject(view, depth, x, y).unwrap_or_else(|| ray.at(distance));
	let normal = estimate_normal(view, depth, x, y, point).unwrap_or(-ray.direction);

	Hit {
		pose: Pose::new(point, Quat::from_up(normal)),
		distance: (point - ray.origin).length(),
		kind: HitKind::Depth,
		plane: None,
	}
}

fn estimate_normal(view: &View, depth: &DepthImage, x: f32, y: f32, point: Vec3) -> Option<Vec3> {
	let right = unproject(view, depth, (x + NORMAL_SPAN).min(1.0), y)? - point;
	let down = unproject(view, depth, x, (y + NORMAL_SPAN).min(1.0))? - point;
	let normal = down.cross(right).normalized()?;

	// Face the viewer.
	Some(if normal.dot(view.pose.position - point) < 0.0 {
		-normal
	} else {
		normal
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::image::DepthFormat;
	use crate::image::IDENTITY_AFFINE;
	use crate::math::Fov;
	use crate::types::PlaneLabel;
	use crate::types::PlaneOrientation;

	fn floor(id: u64, height: f32, size: f32) -> Plane {
		Plane {
			id,
			orientation: PlaneOrientation::HorizontalUp,
			label: PlaneLabel::Floor,
			pose: Pose::at(Vec3::new(0.0, height, -2.0)),
			extent: [size, size],
			polygon: Vec::new(),
		}
	}

	fn camera() -> View {
		View {
			pose: Pose::at(Vec3::new(0.0, 1.5, 0.0)),
			projection: Projection::Fov(Fov::symmetric(1.2, 0.75)),
		}
	}

	#[test]
	fn a_ray_down_hits_the_floor_inside_its_outline() {
		let ray = Ray::new(Vec3::new(0.0, 1.5, -2.0), -Vec3::Y).unwrap();
		let hits = hit_planes(ray, &[floor(1, 0.0, 1.0)], HitTargets::PLANES);
		assert_eq!(hits.len(), 1);
		assert_eq!(hits[0].kind, HitKind::Plane);
		assert!((hits[0].distance - 1.5).abs() < 1e-5);
		assert_eq!(hits[0].plane, Some(1));
	}

	#[test]
	fn outside_an_outline_only_infinite_planes_hit() {
		let ray = Ray::new(Vec3::new(3.0, 1.5, -2.0), -Vec3::Y).unwrap();
		let planes = [floor(1, 0.0, 1.0)];
		assert_eq!(hit_planes(ray, &planes, HitTargets::PLANES), []);
		let hits = hit_planes(ray, &planes, HitTargets::INFINITE_PLANES);
		assert_eq!(hits[0].kind, HitKind::InfinitePlane);
	}

	#[test]
	fn planes_are_hit_from_their_front_only() {
		let up = Ray::new(Vec3::new(0.0, -1.0, -2.0), Vec3::Y).unwrap();
		assert_eq!(hit_planes(up, &[floor(1, 0.0, 1.0)], HitTargets::ALL), []);
	}

	#[test]
	fn requests_sort_nearest_first_and_truncate() {
		let request = HitTestRequest::ray(Ray::new(Vec3::new(0.0, 3.0, -2.0), -Vec3::Y).unwrap())
			.max_results(1);
		let planes = [floor(1, 0.0, 4.0), floor(2, 1.0, 4.0)];
		let hits = hit_test(&request, &[], &planes, None);
		assert_eq!(hits.len(), 1);
		assert_eq!(hits[0].plane, Some(2));
	}

	#[test]
	fn a_screen_tap_at_the_centre_hits_what_the_camera_faces() {
		let mut view = camera();
		view.pose.orientation =
			Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -std::f32::consts::FRAC_PI_4);
		let hits = hit_test(
			&HitTestRequest::screen(0.5, 0.5),
			&[view],
			&[floor(1, 0.0, 10.0)],
			None,
		);
		assert!((hits[0].pose.position.z + 1.5).abs() < 1e-4);
	}

	#[test]
	fn a_ray_stops_at_a_wall_in_the_depth_picture() {
		// Every pixel is 2 m away: a wall facing the camera.
		let data: Vec<u8> = std::iter::repeat_n(2.0_f32.to_le_bytes(), 64 * 48)
			.flatten()
			.collect();
		let depth = DepthImage {
			width: 64,
			height: 48,
			format: DepthFormat::Meters32,
			data: data.into(),
			row_stride: 64 * 4,
			confidence: None,
			display_transform: IDENTITY_AFFINE,
		};
		let view = camera();
		let ray = view.ray(0.5, 0.5).unwrap();
		let hit = hit_depth(ray, &view, &depth).unwrap();
		assert!((hit.distance - 2.0).abs() < 0.03, "{}", hit.distance);
		assert!(hit.pose.transform_vector(Vec3::Y).z > 0.99);
	}

	#[test]
	fn points_project_back_where_they_came_from() {
		let fov = Fov {
			left: -0.7,
			right: 0.6,
			up: 0.5,
			down: -0.65,
		};
		for projection in [
			Projection::Fov(fov),
			Projection::Matrix(fov.projection(0.1, 50.0)),
		] {
			let direction = projection.direction(0.3, 0.8);
			let (x, y) = project(projection, direction * 3.0).unwrap();
			assert!((x - 0.3).abs() < 1e-4 && (y - 0.8).abs() < 1e-4);
		}
	}
}
