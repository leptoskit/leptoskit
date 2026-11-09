//! Rendering imports preserve the versioned guest ABI and tree invariants.

use leptoskit_log::Excerpt;
use leptoskit_native::NativeKind;
use wasmi::Caller;
use wasmi::Linker;

use super::HostNode;
use super::HostState;
use super::MAX_NODE_PROPERTY_BYTES;
use super::MAX_RENDER_PROPERTY_BYTES;
use super::MAX_TREE_DEPTH;
use super::PendingCommit;
use super::WireNode;
use super::caller_memory;
use super::host_trap;
use super::in_import;

const NATIVE_KINDS: &[NativeKind] = &[
	NativeKind::View,
	NativeKind::Text,
	NativeKind::TextRun,
	NativeKind::Image,
	NativeKind::ScrollView,
	NativeKind::TextInput,
	NativeKind::Button,
	NativeKind::Switch,
	NativeKind::Checkbox,
	NativeKind::Radio,
	NativeKind::Slider,
	NativeKind::LinearProgressIndicator,
	NativeKind::ActivityIndicator,
	NativeKind::PasswordInput,
	NativeKind::Pressable,
	NativeKind::Vector,
	NativeKind::Rive,
	NativeKind::Scene3d,
	NativeKind::Video,
	NativeKind::Camera,
];

/// Links the rendering imports. Each names itself in front of its traps, so
/// a failure reads `node_set_props: …` rather than only what went wrong.
pub(super) fn link(linker: &mut Linker<HostState>) -> Result<(), wasmi::errors::LinkerError> {
	linker.func_wrap(
		"leptoskit",
		"node_create",
		|caller: Caller<'_, HostState>, id: i64, kind: i32| {
			node_create(caller, id, kind).map_err(|error| in_import("node_create", error))
		},
	)?;
	linker.func_wrap(
		"leptoskit",
		"node_set_props",
		|caller: Caller<'_, HostState>, id: i64, ptr: i32, len: i32| {
			node_set_props(caller, id, ptr, len).map_err(|error| in_import("node_set_props", error))
		},
	)?;
	linker.func_wrap(
		"leptoskit",
		"node_insert",
		|caller: Caller<'_, HostState>, parent: i64, child: i64, index: i32| {
			node_insert(caller, parent, child, index)
				.map_err(|error| in_import("node_insert", error))
		},
	)?;
	linker.func_wrap(
		"leptoskit",
		"node_remove",
		|caller: Caller<'_, HostState>, parent: i64, child: i64| {
			node_remove(caller, parent, child).map_err(|error| in_import("node_remove", error))
		},
	)?;
	linker.func_wrap(
		"leptoskit",
		"node_delete",
		|caller: Caller<'_, HostState>, id: i64| {
			node_delete(caller, id).map_err(|error| in_import("node_delete", error))
		},
	)?;
	linker.func_wrap("leptoskit", "commit", |caller: Caller<'_, HostState>| {
		commit(caller).map_err(|error| in_import("commit", error))
	})?;
	Ok(())
}

fn node_create(mut caller: Caller<'_, HostState>, id: i64, kind: i32) -> Result<(), wasmi::Error> {
	let state = caller.data_mut();
	state.budget.charge(256)?;
	state.check_nodes(usize::from(!state.nodes.contains_key(&id)))?;
	let kind = NATIVE_KINDS
		.get(kind as u32 as usize)
		.copied()
		.unwrap_or(NativeKind::View);
	state.ensure_node(id);

	// A different widget kind replaces the node and everything below it.
	if state
		.kinds
		.insert(id, kind)
		.is_some_and(|previous| previous != kind)
	{
		state.dirty_structure.insert(id);
	}

	Ok(())
}

fn node_set_props(
	mut caller: Caller<'_, HostState>,
	id: i64,
	ptr: i32,
	len: i32,
) -> Result<(), wasmi::Error> {
	let len = len as u32 as usize;
	let start = ptr as u32 as usize;
	let memory = caller_memory(&mut caller);

	if len != 0 {
		let memory = memory.ok_or_else(|| host_trap("the module does not export memory"))?;

		if start
			.checked_add(len)
			.is_none_or(|end| end > memory.data(&caller).len())
		{
			return Err(host_trap(format!(
				"props range {start}+{len} is out of bounds"
			)));
		}
	}

	let state = caller.data_mut();

	if len > MAX_NODE_PROPERTY_BYTES {
		return Err(state
			.budget
			.reject("render property limit exceeded; reload the guest"));
	}

	state.budget.charge(256 + len)?;
	state.check_nodes(usize::from(!state.nodes.contains_key(&id)))?;
	let old_bytes = state.nodes.get(&id).map_or(0, |node| node.property_bytes);
	let property_bytes = state.budget.property_bytes - old_bytes + len;

	if property_bytes > MAX_RENDER_PROPERTY_BYTES {
		return Err(state
			.budget
			.reject("render retained property limit exceeded; reload the guest"));
	}

	let bytes = if len == 0 {
		&[][..]
	} else {
		memory
			.ok_or_else(|| host_trap("the module does not export memory"))?
			.data(&caller)
			.get(start..start.saturating_add(len))
			.ok_or_else(|| host_trap(format!("props range {start}+{len} is out of bounds")))?
	};

	let mut wire: WireNode = serde_json::from_slice(bytes)
		.map_err(|error| props_error(id, caller.data().kinds.get(&id).copied(), bytes, &error))?;
	let mut rive_bytes = if wire.props.rive.is_some() { len } else { 0 };
	let mut retained_bytes = len;

	if let Some(inputs) = wire.rive_inputs.take() {
		if wire.props.rive.is_some() || caller.data().kinds.get(&id) != Some(&NativeKind::Rive) {
			return Err(host_trap(
				"rive_inputs requires an existing Rive node and no replacement asset",
			));
		}

		let previous = caller.data().nodes.get(&id).ok_or_else(|| {
			host_trap("rive_inputs has no previously installed asset on this node")
		})?;
		rive_bytes = previous.rive_bytes;
		retained_bytes += rive_bytes;

		if retained_bytes > MAX_NODE_PROPERTY_BYTES {
			return Err(caller.data_mut().budget.reject(
				"render property limit exceeded by retained Rive scene; reload the guest",
			));
		}

		if property_bytes + rive_bytes > MAX_RENDER_PROPERTY_BYTES {
			return Err(caller
				.data_mut()
				.budget
				.reject("render retained property limit exceeded; reload the guest"));
		}

		let mut content = previous.wire.props.rive.clone().ok_or_else(|| {
			host_trap("rive_inputs has no previously installed asset on this node")
		})?;
		content
			.replace_inputs(inputs)
			.map_err(|error| host_trap(error.to_string()))?;
		wire.props.rive = Some(content);
	}

	let state = caller.data_mut();
	// A `DevTools` guest names where a node was created only when it renders
	// the node in full; a bound value changing in place sends no source, so
	// the node keeps the one it had.
	if wire.source.is_none()
		&& let Some(previous) = state.nodes.get(&id)
	{
		wire.source.clone_from(&previous.wire.source);
		wire.component.clone_from(&previous.wire.component);
	}
	state.ensure_node(id);
	state.nodes.insert(
		id,
		HostNode {
			wire,
			property_bytes: retained_bytes,
			rive_bytes,
		},
	);
	state.dirty_props.insert(id);
	state.budget.property_bytes = property_bytes + retained_bytes - len;
	Ok(())
}

fn node_insert(
	mut caller: Caller<'_, HostState>,
	parent: i64,
	child: i64,
	index: i32,
) -> Result<(), wasmi::Error> {
	let state = caller.data_mut();
	let shifted = state
		.children
		.get(&parent)
		.map_or(0, Vec::len)
		.saturating_sub(index as u32 as usize);
	state
		.budget
		.charge(256 + 8 * (state.sibling_count(child) + shifted))?;
	// Walk up from the parent: finding the child means the insert would close
	// a cycle. The walk is bounded, so a hostile chain cannot stall the host.
	let mut ancestor = Some(parent);
	let mut depth = 0usize;

	while let Some(id) = ancestor {
		if id == child {
			return Err(host_trap(format!(
				"node_insert({parent}, {child}) would make node {child} its own ancestor"
			)));
		}

		depth += 1;

		if depth >= MAX_TREE_DEPTH {
			return Err(host_trap(format!(
				"node_insert({parent}, {child}) would nest deeper than {MAX_TREE_DEPTH} levels"
			)));
		}

		ancestor = state.parents.get(&id).copied();
	}

	let additional = usize::from(!state.nodes.contains_key(&parent))
		+ usize::from(!state.nodes.contains_key(&child));
	state.check_nodes(additional)?;
	state.detach(child);
	state.ensure_node(parent);
	state.ensure_node(child);
	let siblings = state.children.entry(parent).or_default();
	let index = (index as u32 as usize).min(siblings.len());
	siblings.insert(index, child);
	state.parents.insert(child, parent);
	state.roots.remove(&child);
	state.dirty_structure.insert(parent);
	Ok(())
}

fn node_remove(
	mut caller: Caller<'_, HostState>,
	parent: i64,
	child: i64,
) -> Result<(), wasmi::Error> {
	let state = caller.data_mut();
	state.budget.charge(256 + 8 * state.sibling_count(child))?;

	if state.parents.get(&child) == Some(&parent) {
		state.detach(child);
	}

	Ok(())
}

fn node_delete(mut caller: Caller<'_, HostState>, id: i64) -> Result<(), wasmi::Error> {
	let state = caller.data_mut();
	state.budget.charge(
		256 + 8 * (state.sibling_count(id) + state.children.get(&id).map_or(0, Vec::len)),
	)?;
	state.detach(id);

	for child in state.children.remove(&id).into_iter().flatten() {
		state.parents.remove(&child);
		state.roots.insert(child);
	}

	state.roots.remove(&id);
	state.dirty_props.remove(&id);
	state.dirty_structure.remove(&id);

	if let Some(node) = state.nodes.remove(&id) {
		state.budget.property_bytes -= node.property_bytes;
	}

	state.kinds.remove(&id);
	Ok(())
}

fn commit(mut caller: Caller<'_, HostState>) -> Result<(), wasmi::Error> {
	let state = caller.data_mut();
	state.budget.charge(256)?;

	let root = match state.roots.len() {
		0 => return Ok(()),
		1 => *state.roots.first().expect("one root"),
		_ => {
			let many: Vec<i64> = state.roots.iter().take(9).copied().collect();
			return Err(excess_roots(&many, state.roots.len()));
		}
	};

	// Only the nodes changed since the last commit are rebuilt, unless a
	// commit is already waiting or the root itself changed.
	if state.pending.is_none()
		&& let Some(patches) = state.plan_patches(root)?
	{
		state.pending = Some(PendingCommit::Patch(patches));
		state.commits = state.commits.saturating_add(1);
		state.clear_dirty();

		return Ok(());
	}

	let pending = state.assemble_checked(root)?;
	state.pending = Some(pending);
	state.commits = state.commits.saturating_add(1);
	state.clear_dirty();
	Ok(())
}

/// Why a node's props did not decode, as the trap's message and an error
/// record: the node and its kind, the field, the decoder's error, and the
/// bytes around where it stopped, at most [`leptoskit_log::MAX_EXCERPT_BYTES`]
/// of them.
fn props_error(
	id: i64,
	kind: Option<NativeKind>,
	bytes: &[u8],
	error: &serde_json::Error,
) -> wasmi::Error {
	let field = failing_field(bytes);
	let excerpt = if error.line() > 0 {
		Excerpt::at_line_column(bytes, error.line(), error.column())
	} else {
		// A flattened field's error has no position: find its key instead.
		let at = field
			.as_deref()
			.and_then(|field| field_offset(bytes, field))
			.unwrap_or(0);
		Excerpt::around(bytes, at)
	};
	let kind = kind.map_or_else(|| "not yet created".to_owned(), |kind| kind.to_string());
	let field = field.map_or_else(String::new, |field| format!(" at `{field}`"));
	let message =
		format!("node {id} ({kind}) sent props that do not decode{field}: {error}; {excerpt}");
	log::error!(target: leptoskit_log::target::RENDER, "node_set_props: {message}");
	host_trap(message)
}

/// The path of the field `bytes` fail to decode at, such as `style.flex_grow`.
/// The node's flattened widget props report no path of their own, so those
/// are decoded on their own when the whole node names none.
fn failing_field(bytes: &[u8]) -> Option<String> {
	fn path<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Option<String> {
		let mut decoder = serde_json::Deserializer::from_slice(bytes);
		let path = serde_path_to_error::deserialize::<_, T>(&mut decoder)
			.err()?
			.path()
			.to_string();
		(path != "." && path != "?").then_some(path)
	}

	path::<WireNode>(bytes).or_else(|| path::<leptoskit_native::NativeProps>(bytes))
}

/// Where the key of the last segment of `field` appears in `bytes`.
fn field_offset(bytes: &[u8], field: &str) -> Option<usize> {
	let name = field.rsplit('.').next()?.split('[').next()?;
	let key = format!("\"{name}\"");
	bytes
		.windows(key.len())
		.position(|window| window == key.as_bytes())
		.map(|at| at + key.len())
}

fn excess_roots(roots: &[i64], count: usize) -> wasmi::Error {
	let shown: Vec<String> = roots.iter().take(8).map(i64::to_string).collect();
	let more = if count > shown.len() { ", …" } else { "" };

	host_trap(format!(
		"commit found {} parentless nodes ({}{more}); exactly one root may be committed, so \
		 attach or delete the others first",
		count,
		shown.join(", ")
	))
}
