//! Mutation batches and the recording backend that external widget hosts
//! consume.

use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::MutationList;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_yoga::NodeId;

/// Three distinct delete mutations, for widgets of one committed tree.
fn deletes() -> [Mutation; 3] {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&View().children([View(), View(), View()]));
	let ids: Vec<NodeId> = surface.children(surface.root().unwrap()).to_vec();
	[0, 1, 2].map(|index| Mutation::Delete { id: ids[index] })
}

#[test]
fn mutation_lists_keep_application_order_across_extension() {
	let [a, b, c] = deletes();
	let mut first = MutationList::new();
	assert!(first.is_empty());
	assert_eq!(first.len(), 0);

	first.push(a);
	let mut second = MutationList::new();
	second.push(b);
	second.push(c);
	first.extend(second.clone());

	assert!(!first.is_empty());
	assert_eq!(first.len(), 3);
	assert_eq!(&first.mutations()[1..], second.mutations());
	let owned = first.clone().into_mutations();
	assert_eq!(owned.as_slice(), first.mutations());
}

#[test]
fn taking_the_log_transfers_each_commit_exactly_once() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&View().child(Text("Hello")));

	let first = surface.backend_mut().take_log();
	assert!(
		first
			.iter()
			.any(|mutation| matches!(mutation, Mutation::Create { .. }))
	);
	assert_eq!(surface.backend().log(), []);
	assert_eq!(surface.backend_mut().take_log(), [] as [Mutation; 0]);

	surface.set_root(&View().child(Text("Changed")));
	let second = surface.backend_mut().take_log();
	assert_ne!(second, [] as [Mutation; 0]);
	assert!(
		second
			.iter()
			.all(|mutation| !matches!(mutation, Mutation::Create { .. })),
		"an unchanged structure updates props without recreating widgets: {second:?}"
	);
}

#[test]
fn the_recording_backend_debug_output_names_its_log() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&View());
	let debug = format!("{:?}", surface.backend());

	assert!(debug.starts_with("HeadlessBackend"));
	assert!(debug.contains("log"));
}

/// A backend that implements only the required methods, so the trait's
/// default forwarding is what runs.
#[derive(Default)]
struct MinimalBackend {
	applied: Vec<&'static str>,
}

impl NativeBackend for MinimalBackend {
	fn create(
		&mut self,
		_id: NodeId,
		_kind: leptoskit_native::NativeKind,
		_props: &leptoskit_native::NativeProps,
	) {
		self.applied.push("create");
	}

	fn update_props(&mut self, _id: NodeId, _props: &leptoskit_native::NativeProps) {
		self.applied.push("update_props");
	}

	fn update_layout(&mut self, _id: NodeId, _x: f32, _y: f32, _width: f32, _height: f32) {
		self.applied.push("update_layout");
	}

	fn insert_child(&mut self, _parent: NodeId, _child: NodeId, _index: usize) {
		self.applied.push("insert_child");
	}

	fn remove_child(&mut self, _parent: NodeId, _child: NodeId) {
		self.applied.push("remove_child");
	}

	fn delete(&mut self, _id: NodeId) {
		self.applied.push("delete");
	}

	fn text_measurer(&self) -> leptoskit_native::SharedTextMeasurer {
		std::sync::Arc::new(leptoskit_native::ApproxTextMeasurer::new())
	}
}

#[test]
fn the_trait_defaults_forward_each_mutation_and_report_no_error() {
	let mut surface = NativeSurface::new(MinimalBackend::default());
	surface.set_root(&View().child(Text("Hello")));

	let applied = &surface.backend().applied;
	assert!(applied.contains(&"create"), "{applied:?}");
	assert!(applied.contains(&"insert_child"), "{applied:?}");
	assert!(applied.contains(&"update_layout"), "{applied:?}");

	surface.backend_mut().limit_mutation_log(64);
	assert_eq!(surface.backend().render_error(), None);
}
