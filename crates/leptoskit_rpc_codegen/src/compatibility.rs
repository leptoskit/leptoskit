//! Conservative comparison of the published wire contract. This compares
//! declarations, not authorization policy, handler behavior, or deployment
//! compatibility. A changed schema needs review even if the change might be
//! safe for a particular client.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::ApiFunction;
use crate::ApiKind;
use crate::ApiSpec;

impl ApiSpec {
	/// Changes an installed client needs reviewed before this spec is replaced
	/// by `current`. New operations and optional arguments are allowed;
	/// schemas must retain their shape and referenced definitions.
	///
	/// Unresolved or externally defined schema references cannot establish
	/// compatibility and are reported. This is a structural check: it does
	/// not prove unchanged handler behavior, authorization, or client support
	/// for the schemas. Use generator diagnostics for target-language fidelity.
	#[must_use]
	pub fn breaking_changes(&self, current: &Self) -> Vec<String> {
		breaking_changes(self, current)
	}
}

fn breaking_changes(previous: &ApiSpec, current: &ApiSpec) -> Vec<String> {
	let mut breaks = Vec::new();
	ambiguities(previous, "previous", &mut breaks);
	ambiguities(current, "current", &mut breaks);

	if previous.live_path != current.live_path
		&& (!previous.channels.is_empty()
			|| previous
				.functions
				.iter()
				.any(|function| function.kind != ApiKind::Call))
	{
		breaks.push(format!(
			"live endpoint moved from {} to {}",
			previous.live_path, current.live_path
		));
	}

	for old in &previous.functions {
		let Some(new) = current
			.functions
			.iter()
			.find(|function| function.name == old.name)
		else {
			breaks.push(format!("removed `{}`", old.name));
			continue;
		};

		if old.kind != new.kind {
			breaks.push(format!(
				"`{}` changed from a {} to a {}",
				old.name,
				old.kind.as_str(),
				new.kind.as_str()
			));
		}

		if old.path != new.path || old.method != new.method {
			breaks.push(format!(
				"`{}` moved from {} {} to {} {}",
				old.name, old.method, old.path, new.method, new.path
			));
		}

		if old.input != new.input || old.output != new.output {
			breaks.push(format!("`{}` changed its encoding", old.name));
		}

		if old.error_content_type != new.error_content_type {
			breaks.push(format!("`{}` changed its error encoding", old.name));
		}

		if !same_schema(&old.output_schema, &new.output_schema, previous, current) {
			breaks.push(format!(
				"`{}` result schema changed or has unresolved references",
				old.name
			));
		}

		parameters(previous, current, old, new, &mut breaks);

		for scope in &new.scopes {
			if !old.scopes.contains(scope) {
				breaks.push(format!("`{}` now requires the `{scope}` scope", old.name));
			}
		}
	}

	channels(previous, current, &mut breaks);

	breaks
}

fn parameters(
	previous: &ApiSpec,
	current: &ApiSpec,
	old: &ApiFunction,
	new: &ApiFunction,
	breaks: &mut Vec<String>,
) {
	for param in &old.params {
		if let Some(candidate) = new
			.params
			.iter()
			.find(|candidate| candidate.name == param.name)
		{
			if !param.is_required() && candidate.is_required() {
				breaks.push(format!(
					"`{}` now requires its `{}` argument",
					old.name, param.name
				));
			}

			if !same_schema(&param.schema, &candidate.schema, previous, current) {
				breaks.push(format!(
					"`{}` argument `{}` schema changed or has unresolved references",
					old.name, param.name
				));
			}
		} else {
			breaks.push(format!(
				"`{}` removed its `{}` argument",
				old.name, param.name
			));
		}
	}

	for param in &new.params {
		let was = old
			.params
			.iter()
			.find(|candidate| candidate.name == param.name);

		if was.is_none() && param.is_required() {
			breaks.push(format!(
				"`{}` added the required `{}` argument",
				old.name, param.name
			));
		}
	}
}

fn channels(previous: &ApiSpec, current: &ApiSpec, breaks: &mut Vec<String>) {
	for old in &previous.channels {
		let Some(new) = current
			.channels
			.iter()
			.find(|channel| channel.name == old.name)
		else {
			breaks.push(format!("removed channel `{}`", old.name));
			continue;
		};

		if old.fields != new.fields {
			breaks.push(format!(
				"channel `{}` changed its join field order",
				old.name
			));
		}
		for (kind, before, after) in [
			("join arguments", &old.args_schema, &new.args_schema),
			("messages", &old.message_schema, &new.message_schema),
			("presence", &old.presence_schema, &new.presence_schema),
		] {
			if !same_schema(before, after, previous, current) {
				breaks.push(format!(
					"channel `{}` {kind} schema changed or has unresolved references",
					old.name
				));
			}
		}
	}
}

fn ambiguities(spec: &ApiSpec, label: &str, breaks: &mut Vec<String>) {
	let mut functions = BTreeSet::new();
	let mut routes = BTreeSet::new();

	for function in &spec.functions {
		if !functions.insert(&function.name) {
			breaks.push(format!("{label} spec repeats function `{}`", function.name));
		}

		if function.kind.is_callable_over_http()
			&& !routes.insert((&function.method, &function.path))
		{
			breaks.push(format!(
				"{label} spec repeats route {} {}",
				function.method, function.path
			));
		}
		let mut params = BTreeSet::new();

		for param in &function.params {
			if !params.insert(&param.name) {
				breaks.push(format!(
					"{label} spec repeats `{}` argument `{}`",
					function.name, param.name
				));
			}
		}
	}

	let mut channels = BTreeSet::new();

	for channel in &spec.channels {
		if !channels.insert(&channel.name) {
			breaks.push(format!("{label} spec repeats channel `{}`", channel.name));
		}
	}
}

/// Follow shared definitions without expanding recursive types indefinitely.
/// Equal `$ref` text alone says nothing about the referenced type's shape.
fn same_schema(before: &Value, after: &Value, previous: &ApiSpec, current: &ApiSpec) -> bool {
	let mut seen = BTreeSet::new();
	let mut pending = vec![(before, after)];

	while let Some((left, right)) = pending.pop() {
		if left != right {
			return false;
		}

		let mut references = Vec::new();
		let mut valid = true;
		crate::spec::visit_schema(left, &mut |schema| {
			if !schema.is_object() && !schema.is_boolean() {
				valid = false;
			}

			// Resource identifiers and dynamic references need a full schema
			// resolver. Never infer compatibility by ignoring their behavior.
			if ["$id", "$dynamicRef", "$recursiveRef"]
				.iter()
				.any(|key| schema.get(key).is_some())
			{
				valid = false;
			}
			if let Some(reference) = schema.get("$ref") {
				if let Some(reference) = reference.as_str() {
					references.push(reference.to_owned());
				} else {
					valid = false;
				}
			}
		});

		if !valid {
			return false;
		}

		for reference in references {
			if !seen.insert(reference.clone()) {
				continue;
			}

			let Some(left) = resolve(&reference, before, previous) else {
				return false;
			};
			let Some(right) = resolve(&reference, after, current) else {
				return false;
			};
			pending.push((left, right));
		}
	}

	true
}

fn resolve<'a>(reference: &str, root: &'a Value, spec: &'a ApiSpec) -> Option<&'a Value> {
	let pointer = reference.strip_prefix('#')?;

	if let Some(value) = root.pointer(pointer) {
		return Some(value);
	}

	let rest = pointer
		.strip_prefix("/components/schemas/")
		.or_else(|| pointer.strip_prefix("/$defs/"))?;
	let (name, tail) = rest
		.split_once('/')
		.map_or((rest, None), |(name, tail)| (name, Some(tail)));
	let mut decoded = String::new();
	let mut characters = name.chars();

	while let Some(character) = characters.next() {
		decoded.push(match character {
			'~' => {
				match characters.next()? {
					'0' => '~',
					'1' => '/',
					_ => return None,
				}
			}
			_ => character,
		});
	}

	let value = spec.schemas.get(&decoded)?;

	match tail {
		Some(tail) => value.pointer(&format!("/{tail}")),
		None => Some(value),
	}
}
