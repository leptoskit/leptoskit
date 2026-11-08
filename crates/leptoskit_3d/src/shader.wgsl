// The forward renderer's shaders: lit and unlit meshes (static or
// skinned, with morph targets), the soft shadow under each model, and the
// background (a clear colour or a camera image).
//
// Per-frame data that varies per draw (joint matrices and morph weights)
// lives in a float texture read with textureLoad, never a storage buffer,
// so the same shader runs on WebGL2 and on OpenGL ES drivers without vertex
// storage buffers.

const PI: f32 = 3.14159265;
const NONE: u32 = 0xffffffffu;
const DATA_WIDTH: u32 = 1024u;
const MORPH_WIDTH: u32 = 2048u;

struct Globals {
	view_projection: mat4x4<f32>,
	view: mat4x4<f32>,
	camera: vec4<f32>,
	// xyz: towards the light; w: illuminance.
	light_direction: vec4<f32>,
	// rgb: colour; w: environment strength.
	light_color: vec4<f32>,
	// Irradiance as nine spherical-harmonic coefficients.
	sh: array<vec4<f32>, 9>,
	// x: exposure; y: 1 to encode sRGB in the shader; z: 1 when occlusion
	// depth is bound; w: the environment's last mip.
	params: vec4<f32>,
	// The viewport, in target pixels: x, y, width, height.
	viewport: vec4<f32>,
	// Maps a viewport position (0 to 1, y down) to the occlusion texture.
	occlusion_x: vec4<f32>,
	occlusion_y: vec4<f32>,
	// x: metres per occlusion unit; y: tolerance in metres.
	occlusion_params: vec4<f32>,
}

struct Material {
	base_color: vec4<f32>,
	// rgb: emissive; w: strength.
	emissive: vec4<f32>,
	// metallic, roughness, normal scale, occlusion strength.
	surface: vec4<f32>,
	// x: alpha mode (0 opaque, 1 mask, 2 blend); y: unlit; z: texture mask
	// (1 base, 2 metallic-roughness, 4 normal, 8 occlusion, 16 emissive);
	// w: alpha cutoff as bits.
	flags: vec4<u32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var environment: texture_cube<f32>;
@group(0) @binding(2) var environment_sampler: sampler;
@group(0) @binding(3) var data: texture_2d<f32>;
@group(0) @binding(4) var occlusion: texture_2d<f32>;

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var surface_texture: texture_2d<f32>;
@group(1) @binding(3) var normal_texture: texture_2d<f32>;
@group(1) @binding(4) var occlusion_texture: texture_2d<f32>;
@group(1) @binding(5) var emissive_texture: texture_2d<f32>;
@group(1) @binding(6) var material_sampler: sampler;

@group(2) @binding(0) var morphs: texture_2d<u32>;

struct Instance {
	@location(4) model_0: vec4<f32>,
	@location(5) model_1: vec4<f32>,
	@location(6) model_2: vec4<f32>,
	@location(7) model_3: vec4<f32>,
	// Multiplies base colour; a carries opacity.
	@location(8) tint: vec4<f32>,
	// rgb: emissive override; w: its strength, or below 0 for none.
	@location(9) emissive: vec4<f32>,
	// rgb: rim colour; w: rim strength.
	@location(10) rim: vec4<f32>,
	// x: joints' first texel; y: morph header's texel; z: rim power bits.
	@location(11) params: vec4<u32>,
}

struct Vertex {
	@builtin(vertex_index) index: u32,
	@location(0) position: vec3<f32>,
	@location(1) normal: vec3<f32>,
	@location(2) uv: vec2<f32>,
	@location(3) color: vec4<f32>,
}

struct Skin {
	@location(12) joints: vec4<u32>,
	@location(13) weights: vec4<f32>,
}

struct Varyings {
	@builtin(position) clip: vec4<f32>,
	@location(0) world: vec3<f32>,
	@location(1) normal: vec3<f32>,
	@location(2) uv: vec2<f32>,
	@location(3) color: vec4<f32>,
	@location(4) tint: vec4<f32>,
	@location(5) emissive: vec4<f32>,
	@location(6) rim: vec4<f32>,
	@location(7) @interpolate(flat) rim_power: f32,
	@location(8) view_depth: f32,
}

fn data_texel(index: u32) -> vec4<f32> {
	return textureLoad(data, vec2<u32>(index % DATA_WIDTH, index / DATA_WIDTH), 0);
}

// A joint matrix, stored as the three rows of its affine part.
fn joint(first: u32, index: u32) -> mat4x4<f32> {
	let at = first + index * 3u;
	let r0 = data_texel(at);
	let r1 = data_texel(at + 1u);
	let r2 = data_texel(at + 2u);
	return transpose(mat4x4<f32>(r0, r1, r2, vec4<f32>(0.0, 0.0, 0.0, 1.0)));
}

fn morph_delta(texel: u32) -> vec4<u32> {
	return textureLoad(morphs, vec2<u32>(texel % MORPH_WIDTH, texel / MORPH_WIDTH), 0);
}

// Adds the active morph targets' deltas. The header texel holds the
// primitive's first morph texel, its vertex count, and how many
// (target, weight) pairs follow, two to a texel.
fn morph(header_at: u32, vertex: u32, position: ptr<function, vec3<f32>>, normal: ptr<function, vec3<f32>>) {
	if (header_at == NONE) {
		return;
	}
	let header = data_texel(header_at);
	let base = u32(header.x);
	let count = u32(header.y);
	let pairs = u32(header.z);
	for (var index = 0u; index < pairs; index += 1u) {
		let pair = data_texel(header_at + 1u + index / 2u);
		let target_index = select(u32(pair.x), u32(pair.z), index % 2u == 1u);
		let weight = select(pair.y, pair.w, index % 2u == 1u);
		let packed = morph_delta(base + target_index * count + vertex);
		let position_xy = unpack2x16float(packed.x);
		let position_z_normal_x = unpack2x16float(packed.y);
		let normal_yz = unpack2x16float(packed.z);
		*position += vec3<f32>(position_xy, position_z_normal_x.x) * weight;
		*normal += vec3<f32>(position_z_normal_x.y, normal_yz) * weight;
	}
}

fn finish(world: vec4<f32>, normal: vec3<f32>, vertex: Vertex, instance: Instance) -> Varyings {
	var out: Varyings;
	out.clip = globals.view_projection * world;
	out.world = world.xyz;
	out.normal = normal;
	out.uv = vertex.uv;
	out.color = vertex.color;
	out.tint = instance.tint;
	out.emissive = instance.emissive;
	out.rim = instance.rim;
	out.rim_power = bitcast<f32>(instance.params.z);
	out.view_depth = -(globals.view * world).z;
	return out;
}

@vertex
fn vs_static(vertex: Vertex, instance: Instance) -> Varyings {
	let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
	var position = vertex.position;
	var normal = vertex.normal;
	morph(instance.params.y, vertex.index, &position, &normal);
	let world = model * vec4<f32>(position, 1.0);
	// The inverse transpose of the model's 3×3, up to scale, by cofactors.
	let m = mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz);
	let cofactor = mat3x3<f32>(cross(m[1], m[2]), cross(m[2], m[0]), cross(m[0], m[1]));
	return finish(world, normalize(cofactor * normal), vertex, instance);
}

@vertex
fn vs_skinned(vertex: Vertex, skin: Skin, instance: Instance) -> Varyings {
	var position = vertex.position;
	var normal = vertex.normal;
	morph(instance.params.y, vertex.index, &position, &normal);
	let first = instance.params.x;
	let skinning = joint(first, skin.joints.x) * skin.weights.x
		+ joint(first, skin.joints.y) * skin.weights.y
		+ joint(first, skin.joints.z) * skin.weights.z
		+ joint(first, skin.joints.w) * skin.weights.w;
	let world = skinning * vec4<f32>(position, 1.0);
	let m = mat3x3<f32>(skinning[0].xyz, skinning[1].xyz, skinning[2].xyz);
	let cofactor = mat3x3<f32>(cross(m[1], m[2]), cross(m[2], m[0]), cross(m[0], m[1]));
	return finish(world, normalize(cofactor * normal), vertex, instance);
}

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

// Khronos PBR Neutral: keeps base colours true up to bright highlights,
// which it compresses smoothly.
fn tonemap(color: vec3<f32>) -> vec3<f32> {
	let start = 0.76;
	let desaturation = 0.15;
	let x = min(color.r, min(color.g, color.b));
	let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
	var mapped = color - offset;
	let peak = max(mapped.r, max(mapped.g, mapped.b));
	if (peak < start) {
		return mapped;
	}
	let d = 1.0 - start;
	let new_peak = 1.0 - d * d / (peak + d - start);
	mapped *= new_peak / peak;
	let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
	return mix(mapped, vec3<f32>(new_peak), g);
}

fn irradiance(n: vec3<f32>) -> vec3<f32> {
	let sh = globals.sh;
	return max(vec3<f32>(0.0),
		sh[0].rgb
		+ sh[1].rgb * n.y + sh[2].rgb * n.z + sh[3].rgb * n.x
		+ sh[4].rgb * n.x * n.y + sh[5].rgb * n.y * n.z
		+ sh[6].rgb * (3.0 * n.z * n.z - 1.0)
		+ sh[7].rgb * n.x * n.z + sh[8].rgb * (n.x * n.x - n.y * n.y));
}

// Karis's fit of the split-sum environment BRDF, for mobile.
fn environment_brdf(specular: vec3<f32>, roughness: f32, n_dot_v: f32) -> vec3<f32> {
	let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
	let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
	let r = roughness * c0 + c1;
	let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
	let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
	return specular * ab.x + ab.y;
}

fn distribution(n_dot_h: f32, alpha: f32) -> f32 {
	let a2 = alpha * alpha;
	let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
	return a2 / (PI * d * d);
}

fn visibility(n_dot_v: f32, n_dot_l: f32, alpha: f32) -> f32 {
	let a2 = alpha * alpha;
	let v = n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - a2) + a2);
	let l = n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - a2) + a2);
	return 0.5 / max(v + l, 1.0e-5);
}

// A tangent frame from screen-space derivatives (Schüler), so normal maps
// need no tangent attribute.
fn perturb(normal: vec3<f32>, position: vec3<f32>, uv: vec2<f32>, sample_value: vec3<f32>) -> vec3<f32> {
	let dp1 = dpdx(position);
	let dp2 = dpdy(position);
	let duv1 = dpdx(uv);
	let duv2 = dpdy(uv);
	let dp2perp = cross(dp2, normal);
	let dp1perp = cross(normal, dp1);
	let t = dp2perp * duv1.x + dp1perp * duv2.x;
	let b = dp2perp * duv1.y + dp1perp * duv2.y;
	let scale = inverseSqrt(max(max(dot(t, t), dot(b, b)), 1.0e-12));
	let frame = mat3x3<f32>(t * scale, b * scale, normal);
	return normalize(frame * sample_value);
}

fn occluded(position: vec4<f32>, view_depth: f32) -> bool {
	if (globals.params.z < 0.5) {
		return false;
	}
	let viewport = (position.xy - globals.viewport.xy) / globals.viewport.zw;
	let point = vec3<f32>(viewport, 1.0);
	let uv = vec2<f32>(dot(globals.occlusion_x.xyz, point), dot(globals.occlusion_y.xyz, point));
	if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
		return false;
	}
	let size = vec2<f32>(textureDimensions(occlusion));
	let texel = vec2<u32>(min(uv * size, size - vec2<f32>(1.0)));
	let real = textureLoad(occlusion, texel, 0).r * globals.occlusion_params.x;
	return real > 0.0 && view_depth > real + globals.occlusion_params.y;
}

@fragment
fn fs_mesh(in: Varyings, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
	if (occluded(in.clip, in.view_depth)) {
		discard;
	}
	let mask = material.flags.z;
	var base = material.base_color * in.color * in.tint;
	if ((mask & 1u) != 0u) {
		let texel = textureSample(base_texture, material_sampler, in.uv);
		base *= vec4<f32>(srgb_to_linear(texel.rgb), texel.a);
	}
	if (material.flags.x == 1u && base.a < bitcast<f32>(material.flags.w)) {
		discard;
	}
	// Opaque and masked materials ignore their own alpha; an instance's
	// opacity still applies.
	if (material.flags.x != 2u) {
		base.a = in.tint.a;
	}
	var emissive = material.emissive.rgb * material.emissive.w;
	if ((mask & 16u) != 0u) {
		emissive *= srgb_to_linear(textureSample(emissive_texture, material_sampler, in.uv).rgb);
	}
	if (in.emissive.w >= 0.0) {
		emissive = in.emissive.rgb * in.emissive.w;
	}

	var color: vec3<f32>;
	let v = normalize(globals.camera.xyz - in.world);
	var n = normalize(in.normal);
	if (!front) {
		n = -n;
	}
	if (material.flags.y != 0u) {
		color = base.rgb + emissive;
	} else {
		var metallic = material.surface.x;
		var roughness = material.surface.y;
		if ((mask & 2u) != 0u) {
			let surface = textureSample(surface_texture, material_sampler, in.uv);
			roughness *= surface.g;
			metallic *= surface.b;
		}
		if ((mask & 4u) != 0u) {
			let sampled = textureSample(normal_texture, material_sampler, in.uv).xyz * 2.0 - 1.0;
			n = perturb(n, in.world, in.uv, vec3<f32>(sampled.xy * material.surface.z, sampled.z));
		}
		var ao = 1.0;
		if ((mask & 8u) != 0u) {
			ao = 1.0 + material.surface.w * (textureSample(occlusion_texture, material_sampler, in.uv).r - 1.0);
		}
		roughness = clamp(roughness, 0.045, 1.0);
		let alpha = roughness * roughness;
		let diffuse_color = base.rgb * (1.0 - metallic);
		let specular_color = mix(vec3<f32>(0.04), base.rgb, metallic);
		let n_dot_v = clamp(dot(n, v), 1.0e-4, 1.0);

		let l = globals.light_direction.xyz;
		let h = normalize(l + v);
		let n_dot_l = clamp(dot(n, l), 0.0, 1.0);
		let n_dot_h = clamp(dot(n, h), 0.0, 1.0);
		let v_dot_h = clamp(dot(v, h), 0.0, 1.0);
		let fresnel = specular_color + (1.0 - specular_color) * pow(1.0 - v_dot_h, 5.0);
		let specular = distribution(n_dot_h, alpha) * visibility(n_dot_v, n_dot_l, alpha) * fresnel;
		let direct = (diffuse_color * (1.0 - fresnel) / PI + specular)
			* globals.light_color.rgb * globals.light_direction.w * n_dot_l;

		let strength = globals.light_color.w;
		let r = reflect(-v, n);
		let prefiltered = textureSampleLevel(environment, environment_sampler, r, roughness * globals.params.w).rgb;
		let ambient = (diffuse_color * irradiance(n) + prefiltered * environment_brdf(specular_color, roughness, n_dot_v))
			* strength * ao;
		color = direct + ambient + emissive;
	}

	// The rim: a glow along the silhouette, over everything else.
	if (in.rim.w > 0.0) {
		let facing = 1.0 - clamp(dot(n, v), 0.0, 1.0);
		color += in.rim.rgb * in.rim.w * pow(facing, max(in.rim_power, 0.01));
	}

	color = tonemap(color * globals.params.x);
	if (globals.params.y > 0.5) {
		color = linear_to_srgb(color);
	}
	let opacity = clamp(base.a, 0.0, 1.0);
	return vec4<f32>(color * opacity, opacity);
}

// The soft shadow under a model: a disc on the ground, darkest at its centre.
struct ShadowInstance {
	// xyz: the disc's centre; w: its opacity.
	@location(0) center: vec4<f32>,
	// x, y: its radii along x and z.
	@location(1) radii: vec4<f32>,
}

struct ShadowVaryings {
	@builtin(position) clip: vec4<f32>,
	@location(0) local: vec2<f32>,
	@location(1) opacity: f32,
	@location(2) view_depth: f32,
}

@vertex
fn vs_shadow(@builtin(vertex_index) index: u32, instance: ShadowInstance) -> ShadowVaryings {
	let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u)) * 2.0 - 1.0;
	// A hair above the ground so it does not fight the floor.
	let world = vec4<f32>(
		instance.center.x + corner.x * instance.radii.x,
		instance.center.y + 0.002,
		instance.center.z + corner.y * instance.radii.y,
		1.0,
	);
	var out: ShadowVaryings;
	out.clip = globals.view_projection * world;
	out.local = corner;
	out.opacity = instance.center.w;
	out.view_depth = -(globals.view * world).z;
	return out;
}

@fragment
fn fs_shadow(in: ShadowVaryings) -> @location(0) vec4<f32> {
	if (occluded(in.clip, in.view_depth)) {
		discard;
	}
	// Solid under the model, fading out over the outer part of the disc.
	let falloff = 1.0 - smoothstep(0.2, 1.0, length(in.local));
	return vec4<f32>(0.0, 0.0, 0.0, in.opacity * falloff);
}
