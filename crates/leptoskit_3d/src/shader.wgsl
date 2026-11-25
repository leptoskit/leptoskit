// The forward renderer's shaders: lit and unlit meshes (static or
// skinned, with morph targets), physically based or banded "toon" light,
// the ink outline around them, and the soft shadow under each model. The
// background (a clear colour or a camera image) is in background.wgsl.
//
// Per-frame data that varies per draw (joint matrices and morph weights)
// lives in a float texture read with textureLoad, never a storage buffer,
// so the same shader runs on WebGL2 and on OpenGL ES drivers without vertex
// storage buffers.

const PI: f32 = 3.14159265;
const NONE: u32 = 0xffffffffu;
const DATA_WIDTH: u32 = 1024u;
const MORPH_WIDTH: u32 = 2048u;
// Cells of the line's wobble per unit of the model's own space.
const WOBBLE_FREQUENCY: f32 = 10.0;
// Hatching lines per unit of the model's own space.
const HATCH_DENSITY: f32 = 60.0;

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
	// x: 1 for toon shading; y: its bands; z: their edges' softness; w: the
	// hard highlight's strength.
	style: vec4<f32>,
	// rgb: the darkest band's tint; w: hatching strength.
	shade: vec4<f32>,
	// x: the outline's width in this view's pixels; y: the inner line's
	// strength; z: extra weight in shade; w: wobble.
	ink: vec4<f32>,
	// xyz: where the wobble's noise is read, from the seed and the boil's
	// step; w: the world size of one of this view's pixels at a clip w of 1.
	ink_noise: vec4<f32>,
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
	// x: joints' first texel; y: morph header's texel; z: rim power bits;
	// w: bit 0 set when the part adds its light to what is behind it, and
	// in its high half log2 of the length of a unit of the mesh's space in
	// metres as a half float (0 for 1 m).
	@location(11) params: vec4<u32>,
	// rgb: the part's ink; w: its line's weight, 0 for none.
	@location(14) ink: vec4<f32>,
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
	@location(9) @interpolate(flat) additive: f32,
	@location(10) @interpolate(flat) ink: vec4<f32>,
	// The position in the model's own space before morphs and skinning, so
	// ink drawn on the surface stays fixed to it.
	@location(11) local: vec3<f32>,
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

// A position in the mesh's own space, measured in metres of the model's
// rest pose, which ink on the surface reads. `params`' high half carries
// `log2(units)` as a half float so quantized and large scales stay normal.
fn local_position(position: vec3<f32>, params: u32) -> vec3<f32> {
	return position * exp2(unpack2x16float(params).y);
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
	out.additive = f32(instance.params.w & 1u);
	out.ink = instance.ink;
	out.local = local_position(vertex.position, instance.params.w);
	return out;
}

// The inverse transpose of `m`, up to positive scale, by cofactors;
// negated for mirrored transforms (negative determinant) so outward
// normals stay outward.
fn cofactor_matrix(m: mat3x3<f32>) -> mat3x3<f32> {
	let c0 = cross(m[1], m[2]);
	let sign = select(1.0, -1.0, dot(m[0], c0) < 0.0);
	return mat3x3<f32>(c0 * sign, cross(m[2], m[0]) * sign, cross(m[0], m[1]) * sign);
}

@vertex
fn vs_static(vertex: Vertex, instance: Instance) -> Varyings {
	let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
	var position = vertex.position;
	var normal = vertex.normal;
	morph(instance.params.y, vertex.index, &position, &normal);
	let world = model * vec4<f32>(position, 1.0);
	let m = mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz);
	let cofactor = cofactor_matrix(m);
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
	let cofactor = cofactor_matrix(m);
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

// A hash of a point, from 0 to 1, without sines (Hoskins).
fn hash(point: vec3<f32>) -> f32 {
	var q = fract(point * 0.1031);
	q += dot(q, q.zyx + 31.32);
	return fract((q.x + q.y) * q.z);
}

// Smooth value noise, from 0 to 1.
fn noise(point: vec3<f32>) -> f32 {
	let i = floor(point);
	let f = fract(point);
	let u = f * f * (3.0 - 2.0 * f);
	let a = mix(hash(i), hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
	let b = mix(hash(i + vec3<f32>(0.0, 1.0, 0.0)), hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
	let c = mix(hash(i + vec3<f32>(0.0, 0.0, 1.0)), hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
	let d = mix(hash(i + vec3<f32>(0.0, 1.0, 1.0)), hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
	return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

// How much the line's width wanders at a point of the model's own space:
// a factor around 1, fixed to the surface, drawn afresh at each boil step.
fn wobble(local: vec3<f32>) -> f32 {
	return 1.0 + globals.ink.w * (noise(local * WOBBLE_FREQUENCY + globals.ink_noise.xyz) * 2.0 - 1.0);
}

// Banded light: the colour, and in w how much of the darkest band the
// point is in. The bands' edges are antialiased over about a pixel, more
// with softness, and the darkest band takes the shade's tint, not black.
// The ambient light is flat (the irradiance's constant term), so it never
// smears the bands, and the highlight is a hard-edged spot.
fn toon_light(base: vec3<f32>, metallic: f32, roughness: f32, n: vec3<f32>, v: vec3<f32>, ao: f32) -> vec4<f32> {
	let l = globals.light_direction.xyz;
	let bands = globals.style.y;
	let n_dot_l = dot(n, l);
	// Steps at whole values of y, so each band is centred on a half.
	let y = (n_dot_l * 0.5 + 0.5) * bands + 0.5;
	let edge = clamp(0.5 * fwidth(y) + 0.5 * globals.style.z, 1.0e-5, 0.5);
	let level = clamp(floor(y) - 1.0 + smoothstep(0.5 - edge, 0.5 + edge, fract(y)), 0.0, bands - 1.0);
	let tone = mix(globals.shade.rgb, vec3<f32>(1.0), level / (bands - 1.0)) * ao;
	let light = globals.light_color.rgb * globals.light_direction.w / PI + globals.sh[0].rgb * globals.light_color.w;
	var color = base * tone * light;
	// Guard on uniform specular strength (keeping fwidth in uniform control
	// flow) and avoid normalize(vec3(0.0)) when directly backlit (l == -v).
	if (globals.style.w > 0.0) {
		let half_vec = l + v;
		let h_len2 = dot(half_vec, half_vec);
		let h = select(vec3<f32>(0.0), half_vec * inverseSqrt(max(h_len2, 1.0e-8)), h_len2 > 1.0e-8);
		let n_dot_h = dot(n, h);
		let spot = 1.0 - 0.15 * roughness * roughness;
		let spot_edge = max(fwidth(n_dot_h), 1.0e-5);
		let highlight = smoothstep(spot - spot_edge, spot + spot_edge, n_dot_h)
			* select(0.0, 1.0, n_dot_l > 0.0 && h_len2 > 1.0e-8);
		color += highlight * globals.style.w * (1.0 - 0.5 * roughness) * mix(vec3<f32>(0.08), base, metallic)
			* globals.light_color.rgb * globals.light_direction.w;
	}
	return vec4<f32>(color, 1.0 - clamp(level, 0.0, 1.0));
}

// Hatching fixed to the model's own space: lines from 0 to 1, fading where
// they would be finer than the pixels can show.
fn hatching(local: vec3<f32>) -> f32 {
	let s = dot(local, vec3<f32>(0.53, 0.75, 0.40)) * HATCH_DENSITY;
	let width = max(fwidth(s), 1.0e-5);
	let distance = abs(fract(s + 0.5) - 0.5);
	let line = 1.0 - smoothstep(0.15 - width, 0.15 + width, distance);
	return line * (1.0 - smoothstep(0.2, 0.45, width));
}

// The inner line where a surface turns away from the viewer, from 0 to 1.
// n·v changes fastest there, so holding the line to a number of pixels of
// its gradient keeps it about half the outline's width, and flat faces seen
// edge on, where it hardly changes, stay clear.
fn inner_line(n_dot_v: f32, local: vec3<f32>, weight: f32) -> f32 {
	let gradient = fwidth(n_dot_v);
	let reach = 0.5 * globals.ink.x * weight * wobble(local) * gradient;
	return globals.ink.y * min(weight, 1.0) * (1.0 - smoothstep(reach, reach + max(gradient, 1.0e-5), n_dot_v));
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
	// The surface's own facing, before a normal map, for the inner line:
	// from the file's normal, whichever way the triangle is wound, since
	// quantizing a mesh turns some of its tiniest triangles over.
	let surface_n_dot_v = abs(dot(n, v));
	let toon = globals.style.x > 0.5;
	var hatch = 0.0;
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
		if (toon) {
			let lit = toon_light(base.rgb, metallic, roughness, n, v, ao);
			color = lit.rgb + emissive;
			if (globals.shade.w > 0.0) {
				hatch = globals.shade.w * lit.w * hatching(in.local) * min(in.ink.w, 1.0);
			}
		} else {
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
	}

	// The rim: a glow along the silhouette, over everything else. Toon
	// shading draws it hard-edged, where the soft rim would reach half its
	// strength.
	let facing = 1.0 - clamp(dot(n, v), 0.0, 1.0);
	let facing_edge = max(fwidth(facing), 1.0e-5);
	if (in.rim.w > 0.0) {
		var rim = pow(facing, max(in.rim_power, 0.01));
		if (toon) {
			let start = pow(0.5, 1.0 / max(in.rim_power, 0.01));
			rim = smoothstep(start - facing_edge, start + facing_edge, facing);
		}
		color += in.rim.rgb * in.rim.w * rim;
	}

	color = tonemap(color * globals.params.x);
	// Ink drawn on the surface, in the part's ink colour exactly: hatching in
	// the darkest band and the inner line. Both conditions are uniform, so
	// the inner line's derivatives are too.
	if (globals.ink.y > 0.0 || globals.shade.w > 0.0) {
		let inner = select(0.0, inner_line(surface_n_dot_v, in.local, in.ink.w), globals.ink.y > 0.0);
		color = mix(color, in.ink.rgb, max(hatch, inner));
	}
	if (globals.params.y > 0.5) {
		color = linear_to_srgb(color);
	}
	let opacity = clamp(base.a, 0.0, 1.0);
	// Premultiplied blending adds the colour and keeps what is behind it by
	// one minus alpha, so an additive part writes no alpha at all.
	return vec4<f32>(color * opacity, opacity * (1.0 - in.additive));
}

// The ink outline: the mesh again with its front faces culled, each vertex
// pushed out along its outline normal (welded at load, so hard edges stay
// closed) by the line's width in pixels, scaled by its depth so the width
// holds on screen. Pushing along the normal rather than across the screen
// also moves the hull behind the surface it outlines, so it never fights it
// for depth.
struct OutlineVertex {
	@builtin(vertex_index) index: u32,
	@location(0) position: vec3<f32>,
	@location(1) normal: vec3<f32>,
}

struct OutlineInstance {
	@location(4) model_0: vec4<f32>,
	@location(5) model_1: vec4<f32>,
	@location(6) model_2: vec4<f32>,
	@location(7) model_3: vec4<f32>,
	@location(11) params: vec4<u32>,
	@location(14) ink: vec4<f32>,
}

struct OutlineVaryings {
	@builtin(position) clip: vec4<f32>,
	@location(0) @interpolate(flat) color: vec3<f32>,
	@location(1) view_depth: f32,
}

// The outline normal with the morph targets' change of normal added, in
// the space `transform`'s cofactors map to.
fn outline_normal(transform: mat3x3<f32>, welded: vec3<f32>, normal: vec3<f32>, rest: vec3<f32>) -> vec3<f32> {
	let cofactor = cofactor_matrix(transform);
	let turned = cofactor * (welded + normal - rest);
	let length_squared = dot(turned, turned);
	// A quantized mesh's tiny scale makes the cofactors tiny too, so only an
	// exact zero has no direction.
	return select(vec3<f32>(0.0), turned * inverseSqrt(length_squared), length_squared > 0.0);
}

// `ink.w` is the instance's line weight and `vertex_weight` the model's own,
// from 0 to 2. An instance without a line collapses outside the view.
fn outline(world: vec4<f32>, normal: vec3<f32>, local: vec3<f32>, ink: vec4<f32>, vertex_weight: f32) -> OutlineVaryings {
	var out: OutlineVaryings;
	out.color = ink.rgb;
	if (ink.w <= 0.0) {
		out.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
		out.view_depth = 0.0;
		return out;
	}
	let shade = 0.5 - 0.5 * dot(normal, globals.light_direction.xyz);
	var width = globals.ink.x * ink.w * (1.0 + globals.ink.z * shade) * wobble(local);
	// Hold a 1-pixel floor at full weight (>= 1), tapering the floor down to 0
	// as `ink.w` fades below 1 so fading a part's line out vanishes smoothly;
	// the model's own per-vertex weight may also taper it away.
	width = max(width, min(ink.w, 1.0)) * vertex_weight;
	let depth = (globals.view_projection * world).w;
	let pushed = vec4<f32>(world.xyz + normal * (width * max(depth, 0.0) * globals.ink_noise.w), 1.0);
	out.clip = globals.view_projection * pushed;
	out.view_depth = -(globals.view * pushed).z;
	return out;
}

@vertex
fn vs_outline_static(vertex: OutlineVertex, instance: OutlineInstance, @location(15) ink_vertex: vec4<f32>) -> OutlineVaryings {
	let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
	var position = vertex.position;
	var normal = vertex.normal;
	morph(instance.params.y, vertex.index, &position, &normal);
	let m = mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz);
	let world_normal = outline_normal(m, ink_vertex.xyz, normal, vertex.normal);
	let local = local_position(vertex.position, instance.params.w);
	return outline(model * vec4<f32>(position, 1.0), world_normal, local, instance.ink, ink_vertex.w + 1.0);
}

@vertex
fn vs_outline_skinned(vertex: OutlineVertex, skin: Skin, instance: OutlineInstance, @location(15) ink_vertex: vec4<f32>) -> OutlineVaryings {
	var position = vertex.position;
	var normal = vertex.normal;
	morph(instance.params.y, vertex.index, &position, &normal);
	let first = instance.params.x;
	let skinning = joint(first, skin.joints.x) * skin.weights.x
		+ joint(first, skin.joints.y) * skin.weights.y
		+ joint(first, skin.joints.z) * skin.weights.z
		+ joint(first, skin.joints.w) * skin.weights.w;
	let m = mat3x3<f32>(skinning[0].xyz, skinning[1].xyz, skinning[2].xyz);
	let world_normal = outline_normal(m, ink_vertex.xyz, normal, vertex.normal);
	let local = local_position(vertex.position, instance.params.w);
	return outline(skinning * vec4<f32>(position, 1.0), world_normal, local, instance.ink, ink_vertex.w + 1.0);
}

// The line's colour, exactly as the style or the part set it: no exposure
// or tonemapping, like a clear colour.
@fragment
fn fs_outline(in: OutlineVaryings) -> @location(0) vec4<f32> {
	if (occluded(in.clip, in.view_depth)) {
		discard;
	}
	var color = in.color;
	if (globals.params.y > 0.5) {
		color = linear_to_srgb(color);
	}
	return vec4<f32>(color, 1.0);
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
