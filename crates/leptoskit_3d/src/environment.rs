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
const CUBE_FACES: u32 = 6;
/// Past this height a normal is too near the pole to build a tangent frame
/// from the vertical.
const POLE: f32 = 0.999;

const ZENITH: Vec3 = Vec3::new(0.50, 0.56, 0.66);
const HORIZON: Vec3 = Vec3::new(0.92, 0.92, 0.95);
const GROUND: Vec3 = Vec3::new(0.22, 0.21, 0.20);
/// How quickly the sky turns from horizon to zenith, and the floor from
/// horizon to ground: below one, most of the change is near the horizon.
const SKY_FALLOFF: f32 = 0.6;
const GROUND_FALLOFF: f32 = 0.35;
/// The key light's direction (not normalized), the tightness of its lobe,
/// and its brightness at the centre.
const KEY: Vec3 = Vec3::new(-0.45, 0.75, 0.48);
const KEY_SHARPNESS: f32 = 48.0;
const KEY_STRENGTH: f32 = 6.0;

/// The real spherical harmonics' constants for bands 0 to 2, in the order
/// of `sh_terms`.
const SH_BASIS: [f32; 9] = [
	0.282_095, 0.488_603, 0.488_603, 0.488_603, 1.092_548, 1.092_548, 0.315_392, 1.092_548,
	0.546_274,
];
/// The irradiance integral's grid over the sphere, in latitude rows and
/// longitude columns.
const IRRADIANCE_ROWS: u32 = 64;
const IRRADIANCE_COLUMNS: u32 = 128;

/// The sky's radiance in a direction: a cool zenith, a bright horizon, a
/// darker floor, and a soft key light up and to one side, so glossy parts
/// show a highlight.
pub(crate) fn radiance(direction: Vec3) -> Vec3 {
	let height = direction.y;
	let sky = if height >= 0.0 {
		HORIZON.lerp(ZENITH, height.powf(SKY_FALLOFF))
	} else {
		HORIZON.lerp(GROUND, (-height).powf(GROUND_FALLOFF))
	};

	let lobe = direction.dot(KEY.normalize()).max(0.0).powf(KEY_SHARPNESS) * KEY_STRENGTH;
	sky + Vec3::splat(lobe)
}

/// The direction through texel (`x`, `y`) of `face`, in the order wgpu
/// reads cube faces: +x, -x, +y, -y, +z, -z.
pub(crate) fn face_direction(face: u32, x: u32, y: u32, size: u32) -> Vec3 {
	let across = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
	let down = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;

	let direction = match face {
		0 => Vec3::new(1.0, -down, -across),
		1 => Vec3::new(-1.0, -down, across),
		2 => Vec3::new(across, 1.0, down),
		3 => Vec3::new(across, -1.0, -down),
		4 => Vec3::new(across, -down, 1.0),
		_ => Vec3::new(-across, -down, -1.0),
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
	let up = if normal.y.abs() < POLE {
		Vec3::Y
	} else {
		Vec3::X
	};

	let tangent = up.cross(normal).normalize();
	let bitangent = normal.cross(tangent);
	let mut sum = Vec3::ZERO;

	let mut weight = 0.0;

	for index in 0..SAMPLES {
		let (spin, height) = hammersley(index, SAMPLES);
		let phi = 2.0 * std::f32::consts::PI * spin;
		let cos_theta = ((1.0 - height) / (1.0 + (alpha * alpha - 1.0) * height)).sqrt();
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
			let texels = (0..CUBE_FACES).flat_map(move |face| {
				(0..size * size)
					.map(move |texel| face_direction(face, texel % size, texel / size, size))
			});

			texels
				.map(|direction| prefiltered(direction, roughness).extend(1.0).to_array())
				.collect()
		})
		.collect()
}

/// Irradiance over π as nine spherical-harmonic coefficients, already
/// scaled by their basis constants and the cosine lobe's band factors, so
/// the shader only multiplies by the polynomial terms.
pub(crate) fn irradiance_sh() -> [[f32; 4]; 9] {
	let mut coefficients = [Vec3::ZERO; 9];
	let (rows, columns) = (IRRADIANCE_ROWS, IRRADIANCE_COLUMNS);
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
			let terms = sh_terms(direction);

			for index in 0..SH_BASIS.len() {
				let basis = SH_BASIS[index] * terms[index];
				coefficients[index] += color * basis * solid;
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
	let mut out = [[0.0; 4]; 9];

	for index in 0..9 {
		let value = coefficients[index] * scale * band[index] * SH_BASIS[index];
		out[index] = [value.x, value.y, value.z, 0.0];
	}

	out
}

/// The spherical harmonics' polynomial terms for bands 0 to 2, in the order
/// the shader evaluates them.
fn sh_terms(Vec3 { x, y, z }: Vec3) -> [f32; 9] {
	[
		1.0,
		y,
		z,
		x,
		x * y,
		y * z,
		3.0 * z * z - 1.0,
		x * z,
		x * x - y * y,
	]
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
		let sharp = prefiltered(KEY.normalize(), 0.0);
		let rough = prefiltered(KEY.normalize(), 1.0);
		assert!(sharp.x > rough.x, "the key light spreads out");
	}
}
