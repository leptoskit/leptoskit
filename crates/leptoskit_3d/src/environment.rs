//! The environment that lights every scene: a neutral studio sky, baked on
//! the CPU into a small prefiltered cubemap and nine spherical-harmonic
//! irradiance coefficients.
//!
//! It is baked once per device and shared by every view. Baking per scene
//! was ruled out: the Solana Seeker's GPU reset under that kind of
//! allocation churn.

use glam::Vec3;

/// The cubemap's widest mip, in texels per face edge.
pub(crate) const SIZE: u32 = 64;
/// Its mips, from mirror-sharp to fully rough.
pub(crate) const MIPS: u32 = 6;
const SAMPLES: u32 = 48;

/// The sky's radiance in a direction: a cool zenith, a bright horizon, a
/// darker floor, and a soft key light up and to one side, so glossy parts
/// show a highlight.
pub(crate) fn radiance(direction: Vec3) -> Vec3 {
	let zenith = Vec3::new(0.50, 0.56, 0.66);
	let horizon = Vec3::new(0.92, 0.92, 0.95);
	let ground = Vec3::new(0.22, 0.21, 0.20);
	let y = direction.y;
	let sky = if y >= 0.0 {
		horizon.lerp(zenith, y.powf(0.6))
	} else {
		horizon.lerp(ground, (-y).powf(0.35))
	};
	let key = Vec3::new(-0.45, 0.75, 0.48).normalize();
	let lobe = direction.dot(key).max(0.0).powf(48.0) * 6.0;
	sky + Vec3::splat(lobe)
}

/// The direction through texel (`x`, `y`) of `face`, in the order wgpu
/// reads cube faces: +x, -x, +y, -y, +z, -z.
pub(crate) fn face_direction(face: u32, x: u32, y: u32, size: u32) -> Vec3 {
	let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
	let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
	let direction = match face {
		0 => Vec3::new(1.0, -v, -u),
		1 => Vec3::new(-1.0, -v, u),
		2 => Vec3::new(u, 1.0, v),
		3 => Vec3::new(u, -1.0, -v),
		4 => Vec3::new(u, -v, 1.0),
		_ => Vec3::new(-u, -v, -1.0),
	};
	direction.normalize()
}

fn hammersley(index: u32, count: u32) -> (f32, f32) {
	(
		index as f32 / count as f32,
		index.reverse_bits() as f32 * 2.328_306_4e-10,
	)
}

/// The sky prefiltered with a GGX lobe of `roughness` around `normal`, with
/// the view along the normal, as the split-sum approximation assumes.
fn prefiltered(normal: Vec3, roughness: f32) -> Vec3 {
	if roughness <= 0.0 {
		return radiance(normal);
	}
	let alpha = roughness * roughness;
	let up = if normal.y.abs() < 0.999 {
		Vec3::Y
	} else {
		Vec3::X
	};
	let tangent = up.cross(normal).normalize();
	let bitangent = normal.cross(tangent);
	let mut sum = Vec3::ZERO;
	let mut weight = 0.0;
	for index in 0..SAMPLES {
		let (u, v) = hammersley(index, SAMPLES);
		let phi = 2.0 * std::f32::consts::PI * u;
		let cos_theta = ((1.0 - v) / (1.0 + (alpha * alpha - 1.0) * v)).sqrt();
		let sin_theta = (1.0 - cos_theta * cos_theta).sqrt();
		let half = tangent * (sin_theta * phi.cos())
			+ bitangent * (sin_theta * phi.sin())
			+ normal * cos_theta;
		let light = 2.0 * normal.dot(half) * half - normal;
		let n_dot_l = normal.dot(light);
		if n_dot_l > 0.0 {
			sum += radiance(light) * n_dot_l;
			weight += n_dot_l;
		}
	}
	if weight > 0.0 {
		sum / weight
	} else {
		radiance(normal)
	}
}

/// Every mip of every face, as RGBA floats, mip by mip and face by face.
pub(crate) fn bake_cubemap() -> Vec<Vec<[f32; 4]>> {
	(0..MIPS)
		.map(|mip| {
			let size = (SIZE >> mip).max(1);
			let roughness = mip as f32 / (MIPS - 1) as f32;
			let mut texels = Vec::with_capacity((size * size * 6) as usize);
			for face in 0..6 {
				for y in 0..size {
					for x in 0..size {
						let color = prefiltered(face_direction(face, x, y, size), roughness);
						texels.push([color.x, color.y, color.z, 1.0]);
					}
				}
			}
			texels
		})
		.collect()
}

/// Irradiance over π as nine spherical-harmonic coefficients, already
/// scaled by their basis constants and the cosine lobe's band factors, so
/// the shader only multiplies by the polynomial terms.
pub(crate) fn irradiance_sh() -> [[f32; 4]; 9] {
	let mut coefficients = [Vec3::ZERO; 9];
	let (rows, columns) = (64, 128);
	let mut total = 0.0;
	for row in 0..rows {
		let theta = (row as f32 + 0.5) / rows as f32 * std::f32::consts::PI;
		let solid = theta.sin();
		for column in 0..columns {
			let phi = (column as f32 + 0.5) / columns as f32 * 2.0 * std::f32::consts::PI;
			let direction = Vec3::new(
				theta.sin() * phi.cos(),
				theta.cos(),
				theta.sin() * phi.sin(),
			);
			let color = radiance(direction);
			let (x, y, z) = (direction.x, direction.y, direction.z);
			let basis = [
				0.282_095,
				0.488_603 * y,
				0.488_603 * z,
				0.488_603 * x,
				1.092_548 * x * y,
				1.092_548 * y * z,
				0.315_392 * (3.0 * z * z - 1.0),
				1.092_548 * x * z,
				0.546_274 * (x * x - y * y),
			];
			for (coefficient, basis) in coefficients.iter_mut().zip(basis) {
				*coefficient += color * basis * solid;
			}
			total += solid;
		}
	}
	let scale = 4.0 * std::f32::consts::PI / total;
	// The cosine lobe's band factors, over π for Lambert.
	let band = [
		1.0,
		2.0 / 3.0,
		2.0 / 3.0,
		2.0 / 3.0,
		0.25,
		0.25,
		0.25,
		0.25,
		0.25,
	];
	let constants = [
		0.282_095, 0.488_603, 0.488_603, 0.488_603, 1.092_548, 1.092_548, 0.315_392, 1.092_548,
		0.546_274,
	];
	let mut out = [[0.0; 4]; 9];
	for index in 0..9 {
		let value = coefficients[index] * scale * band[index] * constants[index];
		out[index] = [value.x, value.y, value.z, 0.0];
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	fn evaluate(sh: &[[f32; 4]; 9], n: Vec3) -> Vec3 {
		let term = |index: usize| Vec3::new(sh[index][0], sh[index][1], sh[index][2]);
		term(0)
			+ term(1) * n.y
			+ term(2) * n.z
			+ term(3) * n.x
			+ term(4) * n.x * n.y
			+ term(5) * n.y * n.z
			+ term(6) * (3.0 * n.z * n.z - 1.0)
			+ term(7) * n.x * n.z
			+ term(8) * (n.x * n.x - n.y * n.y)
	}

	#[test]
	fn faces_point_where_wgpu_reads_them() {
		assert!(face_direction(0, 31, 31, 64).abs_diff_eq(Vec3::X, 0.05));
		assert!(face_direction(2, 31, 31, 64).abs_diff_eq(Vec3::Y, 0.05));
		assert!(face_direction(5, 31, 31, 64).abs_diff_eq(Vec3::NEG_Z, 0.05));
		// The top edge of +z looks 45° up, between +z and +y.
		let top = face_direction(4, 32, 0, 64);
		assert!(top.y > 0.65 && top.z > 0.65, "{top}");
	}

	#[test]
	fn irradiance_is_brighter_from_the_sky_than_the_floor() {
		let sh = irradiance_sh();
		let up = evaluate(&sh, Vec3::Y);
		let down = evaluate(&sh, Vec3::NEG_Y);
		assert!(up.x > down.x * 1.5, "{up} {down}");
		assert!(up.min_element() > 0.0 && down.min_element() > 0.0);
	}

	#[test]
	fn rougher_mips_are_smoother() {
		let key = Vec3::new(-0.45, 0.75, 0.48).normalize();
		let sharp = prefiltered(key, 0.0);
		let rough = prefiltered(key, 1.0);
		assert!(sharp.x > rough.x, "the key light spreads out");
	}
}
