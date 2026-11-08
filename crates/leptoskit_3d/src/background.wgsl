// The background: a full-screen triangle sampling a camera image, drawn
// before the scene. An RGBA image is read as linear light, so pass a view
// in an sRGB format for sRGB-encoded pixels.

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
	let low = color / 12.92;
	let high = pow((color + 0.055) / 1.055, vec3<f32>(2.4));
	return select(high, low, color <= vec3<f32>(0.04045));
}

fn linear_to_srgb(color: vec3<f32>) -> vec3<f32> {
	let low = color * 12.92;
	let high = 1.055 * pow(color, vec3<f32>(1.0 / 2.4)) - 0.055;
	return select(high, low, color <= vec3<f32>(0.0031308));
}

struct Background {
	// Maps a viewport position (0 to 1, y down) to the image's coordinates.
	transform_x: vec4<f32>,
	transform_y: vec4<f32>,
	// x: 0 RGBA, 1 luma and chroma planes; y: 1 to encode sRGB.
	mode: vec4<f32>,
}

@group(0) @binding(0) var<uniform> background: Background;
@group(0) @binding(1) var plane_0: texture_2d<f32>;
@group(0) @binding(2) var plane_1: texture_2d<f32>;
@group(0) @binding(3) var plane_sampler: sampler;

struct BackgroundVaryings {
	@builtin(position) clip: vec4<f32>,
	@location(0) viewport: vec2<f32>,
}

@vertex
fn vs_background(@builtin(vertex_index) index: u32) -> BackgroundVaryings {
	let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
	var out: BackgroundVaryings;
	out.clip = vec4<f32>(corner * 2.0 - 1.0, 1.0, 1.0);
	out.viewport = vec2<f32>(corner.x, 1.0 - corner.y);
	return out;
}

@fragment
fn fs_background(in: BackgroundVaryings) -> @location(0) vec4<f32> {
	let point = vec3<f32>(in.viewport, 1.0);
	let uv = vec2<f32>(dot(background.transform_x.xyz, point), dot(background.transform_y.xyz, point));
	var color: vec3<f32>;
	if (background.mode.x > 0.5) {
		// Full-range BT.601, as ARKit and ARCore deliver camera frames.
		let y = textureSample(plane_0, plane_sampler, uv).r;
		let cbcr = textureSample(plane_1, plane_sampler, uv).rg - vec2<f32>(0.5);
		let srgb = vec3<f32>(
			y + 1.402 * cbcr.y,
			y - 0.344136 * cbcr.x - 0.714136 * cbcr.y,
			y + 1.772 * cbcr.x,
		);
		color = srgb_to_linear(clamp(srgb, vec3<f32>(0.0), vec3<f32>(1.0)));
	} else {
		color = textureSample(plane_0, plane_sampler, uv).rgb;
	}
	if (background.mode.y > 0.5) {
		color = linear_to_srgb(color);
	}
	return vec4<f32>(color, 1.0);
}
