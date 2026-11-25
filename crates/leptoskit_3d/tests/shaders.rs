//! Validates the shaders and translates every entry point for each backend
//! the crate draws with: Metal Shading Language, SPIR-V for Vulkan, HLSL for
//! Direct3D 12, and GLSL ES 3.0 for OpenGL ES and WebGL2, within WebGL2's
//! limits on vertex inputs and varyings. The render tests compile the
//! shaders only for this machine's GPU; this compiles them for all of them,
//! with no GPU.
#![cfg(all(feature = "renderer", not(target_arch = "wasm32")))]

use naga::Binding;
use naga::Expression;
use naga::Function;
use naga::Handle;
use naga::Literal;
use naga::MathFunction;
use naga::ShaderStage;
use naga::TypeInner;
use naga::back::glsl;
use naga::back::hlsl;
use naga::back::msl;
use naga::back::spv;
use naga::valid::Capabilities;
use naga::valid::ModuleInfo;
use naga::valid::ValidationFlags;
use naga::valid::Validator;

const SHADERS: [(&str, &str); 2] = [
	("shader.wgsl", include_str!("../src/shader.wgsl")),
	("background.wgsl", include_str!("../src/background.wgsl")),
];

/// WebGL2's guaranteed `MAX_VERTEX_ATTRIBS`, the lowest of every backend.
const MAX_VERTEX_INPUTS: usize = 16;
/// WebGL2's guaranteed `MAX_VARYING_VECTORS`.
const MAX_VARYINGS: usize = 15;

fn parse(name: &str, source: &str) -> (naga::Module, ModuleInfo) {
	let module = naga::front::wgsl::parse_str(source)
		.unwrap_or_else(|error| panic!("{}", error.emit_to_string_with_path(source, name)));
	// What wgpu grants every device, WebGL2 included: only the packing of
	// halves into 32-bit words.
	let info = Validator::new(
		ValidationFlags::all(),
		Capabilities::SHADER_FLOAT16_IN_FLOAT32,
	)
	.validate(&module)
	.unwrap_or_else(|error| panic!("{}", error.emit_to_string_with_path(source, name)));
	(module, info)
}

/// The user locations a binding or a struct of bindings takes.
fn locations(module: &naga::Module, ty: Handle<naga::Type>, binding: Option<&Binding>) -> Vec<u32> {
	match (binding, &module.types[ty].inner) {
		(Some(Binding::Location { location, .. }), _) => vec![*location],
		(None, TypeInner::Struct { members, .. }) => {
			members
				.iter()
				.flat_map(|member| locations(module, member.ty, member.binding.as_ref()))
				.collect()
		}
		(Some(Binding::BuiltIn(_)) | None, _) => Vec::new(),
	}
}

#[test]
fn shaders_translate_for_every_backend() {
	for (name, source) in SHADERS {
		let (module, info) = parse(name, source);

		msl::write_string(
			&module,
			&info,
			&msl::Options::default(),
			&msl::PipelineOptions::default(),
		)
		.unwrap_or_else(|error| panic!("{name} as MSL: {error}"));
		spv::write_vec(&module, &info, &spv::Options::default(), None)
			.unwrap_or_else(|error| panic!("{name} as SPIR-V: {error}"));
		let mut hlsl_source = String::new();
		hlsl::Writer::new(
			&mut hlsl_source,
			&hlsl::Options::default(),
			&hlsl::PipelineOptions::default(),
		)
		.write(&module, &info, None)
		.unwrap_or_else(|error| panic!("{name} as HLSL: {error}"));

		for entry in &module.entry_points {
			let mut glsl_source = String::new();
			let options = glsl::Options {
				version: glsl::Version::Embedded {
					version: 300,
					is_webgl: true,
				},
				..glsl::Options::default()
			};
			let pipeline = glsl::PipelineOptions {
				shader_stage: entry.stage,
				entry_point: entry.name.clone(),
				multiview: None,
			};
			glsl::Writer::new(
				&mut glsl_source,
				&module,
				&info,
				&options,
				&pipeline,
				naga::proc::BoundsCheckPolicies::default(),
			)
			.and_then(|mut writer| writer.write())
			.unwrap_or_else(|error| panic!("{name}'s {} as GLSL ES 3.0: {error}", entry.name));
		}
	}
}

#[test]
fn vertex_stages_fit_webgl2() {
	let mut vertex_stages = 0;

	for (name, source) in SHADERS {
		let (module, _) = parse(name, source);

		for entry in module
			.entry_points
			.iter()
			.filter(|entry| entry.stage == ShaderStage::Vertex)
		{
			let inputs: Vec<u32> = entry
				.function
				.arguments
				.iter()
				.flat_map(|argument| locations(&module, argument.ty, argument.binding.as_ref()))
				.collect();
			let outputs = entry.function.result.as_ref().map_or(Vec::new(), |result| {
				locations(&module, result.ty, result.binding.as_ref())
			});
			assert!(
				inputs.len() <= MAX_VERTEX_INPUTS
					&& inputs
						.iter()
						.all(|location| (*location as usize) < MAX_VERTEX_INPUTS),
				"{name}'s {} reads {inputs:?}",
				entry.name
			);
			assert!(
				outputs.len() <= MAX_VARYINGS,
				"{name}'s {} writes {outputs:?}",
				entry.name
			);
			vertex_stages += 1;
		}
	}

	// The mesh's (static, skinned, morphed, both), the outline's two, and
	// the background's, shadow's and others: a stage renamed or dropped
	// should be noticed here.
	assert!(vertex_stages >= 6, "{vertex_stages}");
}

fn positive_literal(
	module: &naga::Module,
	expr: Handle<Expression>,
	func: &Function,
) -> Option<f32> {
	match func.expressions[expr] {
		Expression::Literal(Literal::F32(value)) if value > 0.0 => Some(value),
		Expression::Constant(handle) => {
			match module.global_expressions[module.constants[handle].init] {
				Expression::Literal(Literal::F32(value)) if value > 0.0 => Some(value),
				_ => None,
			}
		}
		_ => None,
	}
}

fn literal_f32(module: &naga::Module, expr: Handle<Expression>, func: &Function) -> Option<f32> {
	match func.expressions[expr] {
		Expression::Literal(Literal::F32(value)) => Some(value),
		Expression::Constant(handle) => {
			match module.global_expressions[module.constants[handle].init] {
				Expression::Literal(Literal::F32(value)) => Some(value),
				_ => None,
			}
		}
		_ => None,
	}
}

/// Whether `expr` (or an expression it depends on) floors a width with a
/// positive `max(..., eps)` or `clamp(..., eps, ...)` so `smoothstep(a, b, x)`
/// always has `a < b` even when `fwidth(...) == 0.0`.
fn has_positive_floor(module: &naga::Module, func: &Function, expr: Handle<Expression>) -> bool {
	match func.expressions[expr] {
		Expression::Math {
			fun: MathFunction::Max,
			arg,
			arg1: Some(arg1),
			..
		} => {
			positive_literal(module, arg, func).is_some()
				|| positive_literal(module, arg1, func).is_some()
		}
		Expression::Math {
			fun: MathFunction::Clamp,
			arg1: Some(low),
			..
		} => positive_literal(module, low, func).is_some(),
		Expression::Binary { left, right, .. } => {
			has_positive_floor(module, func, left) || has_positive_floor(module, func, right)
		}
		Expression::Unary { expr, .. } | Expression::Swizzle { vector: expr, .. } => {
			has_positive_floor(module, func, expr)
		}
		_ => false,
	}
}

#[test]
fn smoothstep_bounds_are_strictly_ordered_on_flat_faces() {
	let (module, _) = parse("shader.wgsl", SHADERS[0].1);
	let functions = module
		.functions
		.iter()
		.map(|(_, func)| func)
		.chain(module.entry_points.iter().map(|entry| &entry.function));
	let mut checked = 0;

	for func in functions {
		for (_, expr) in func.expressions.iter() {
			let Expression::Math {
				fun: MathFunction::SmoothStep,
				arg: low,
				arg1: Some(high),
				..
			} = *expr
			else {
				continue;
			};
			checked += 1;
			let ordered_constants = literal_f32(&module, low, func)
				.zip(literal_f32(&module, high, func))
				.is_some_and(|(a, b)| a < b);
			assert!(
				ordered_constants
					|| has_positive_floor(&module, func, low)
					|| has_positive_floor(&module, func, high),
				"smoothstep in {:?} has no positive floor on its bounds (undefined when fwidth == \
				 0)",
				func.name
			);
		}
	}

	assert!(
		checked >= 7,
		"expected all shader smoothsteps, saw {checked}"
	);
}
