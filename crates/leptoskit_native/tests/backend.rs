//! Mutation batches and the recording backend that external widget hosts
//! consume.

use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::MutationList;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeSurface;
use leptoskit_native::RevealRequest;
use leptoskit_native::ScrollAlign;
use leptoskit_native::components::ScrollView;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
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

/// The props mutations of a commit, as `(creates, request)` pairs: whether
/// each one created its widget, and the focus request it carried.
fn focus_requests(log: &[Mutation]) -> Vec<(bool, u64)> {
	log.iter()
		.filter_map(|mutation| {
			match mutation {
				Mutation::Create { props, .. } => Some((true, props.focus_request?)),
				Mutation::UpdateProps { props, .. } => Some((false, props.focus_request?)),
				_ => None,
			}
		})
		.collect()
}

/// A screen with a field that holds the focus request `request`.
fn code_screen(title: &str, request: Option<u64>) -> leptoskit_native::Element {
	View().child(Text(title)).child(
		TextInput()
			.aria_label("Code")
			.with_props(|props| props.focus_request = request),
	)
}

#[test]
fn a_focus_request_reaches_the_backend_once() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&code_screen("Sign in", None));
	assert_eq!(focus_requests(&surface.backend_mut().take_log()), []);

	surface.set_root(&code_screen("Sign in", Some(4)));
	assert_eq!(
		focus_requests(&surface.backend_mut().take_log()),
		[(false, 4)]
	);

	// Something else changes while the field still holds the request.
	surface.set_root(&code_screen("Check your email", Some(4)));
	let unrelated = surface.backend_mut().take_log();
	assert_ne!(unrelated, [] as [Mutation; 0]);
	assert_eq!(focus_requests(&unrelated), []);

	surface.set_root(&code_screen("Check your email", Some(9)));
	assert_eq!(
		focus_requests(&surface.backend_mut().take_log()),
		[(false, 9)]
	);
}

#[test]
fn a_widget_created_with_a_new_request_takes_focus_and_an_old_one_does_not() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&code_screen("Sign in", Some(4)));
	assert_eq!(
		focus_requests(&surface.backend_mut().take_log()),
		[(true, 4)]
	);

	// The field leaves and comes back still holding the request it had.
	surface.set_root(&View().child(Text("Sign in")));
	surface.set_root(&code_screen("Sign in", Some(4)));
	assert_eq!(focus_requests(&surface.backend_mut().take_log()), []);
}

#[test]
fn a_batch_names_the_last_widget_asked_to_take_focus() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&View().children([
		TextInput().aria_label("Email"),
		TextInput().aria_label("Code"),
	]));
	let fields: Vec<NodeId> = surface.children(surface.root().unwrap()).to_vec();
	let asks = |id, request| {
		Mutation::UpdateProps {
			id,
			props: TextInput()
				.with_props(|props| props.focus_request = request)
				.props,
		}
	};

	let mut batch = MutationList::new();
	assert_eq!(batch.focus_target(), None);

	batch.push(asks(fields[0], Some(1)));
	batch.push(asks(fields[1], None));
	assert_eq!(batch.focus_target(), Some(fields[0]));

	batch.push(asks(fields[1], Some(2)));
	assert_eq!(batch.focus_target(), Some(fields[1]));

	batch.push(Mutation::Delete { id: fields[1] });
	assert_eq!(batch.focus_target(), Some(fields[0]));
}

/// The scroll offsets a commit sent the root, as `(x, y, animated)`, with
/// `None` where it was told to forget the last one.
fn scroll_offsets(surface: &mut NativeSurface<HeadlessBackend>) -> Vec<Option<(f32, f32, bool)>> {
	let view = surface.root().unwrap();

	surface
		.backend_mut()
		.take_log()
		.iter()
		.filter_map(|mutation| {
			let Mutation::UpdateProps { id, props } = mutation else {
				return None;
			};

			if *id != view {
				return None;
			}

			Some(
				props
					.scroll_offset
					.map(|offset| (offset.x, offset.y, offset.animated)),
			)
		})
		.collect()
}

/// A 100-tall scroll view of ten 50-tall rows, the row at `index` asking to
/// be revealed.
fn rows(index: usize, reveal: Option<RevealRequest>) -> leptoskit_native::Element {
	ScrollView().class("h-[100px]").children((0..10).map(|row| {
		Text(format!("Row {row}"))
			.class("h-[50px] shrink-0")
			.with_props(|props| props.reveal = reveal.filter(|_| row == index))
	}))
}

fn asked(serial: u64, align: ScrollAlign) -> RevealRequest {
	RevealRequest { serial, align }
}

#[test]
fn a_reveal_scrolls_the_view_around_the_element_once() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(200.0, 100.0);
	surface.set_root(&rows(6, None));
	surface.backend_mut().take_log();

	// Row 6 starts at 300. The view forgets its last offset first, so a
	// renderer scrolls even when it was sent this offset before.
	surface.set_root(&rows(6, Some(asked(3, ScrollAlign::Start))));
	assert!(surface.backend().log().iter().all(|mutation| {
		!matches!(mutation, Mutation::UpdateProps { props, .. } if props.reveal.is_some())
	}));
	assert_eq!(
		scroll_offsets(&mut surface),
		[None, Some((0.0, 300.0, true))]
	);

	// The row keeps the request while something else builds again.
	surface.set_root(&rows(6, Some(asked(3, ScrollAlign::Start))));
	assert_eq!(surface.backend_mut().take_log(), [] as [Mutation; 0]);

	surface.set_root(&rows(6, Some(asked(5, ScrollAlign::End))));
	assert_eq!(
		scroll_offsets(&mut surface),
		[None, Some((0.0, 250.0, true))]
	);
}

#[test]
fn a_reveal_of_something_in_view_moves_nothing() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(200.0, 100.0);
	surface.set_root(&rows(1, Some(asked(2, ScrollAlign::Nearest))));
	assert_eq!(scroll_offsets(&mut surface), []);

	// The person scrolls to 200, which shows rows 4 and 5.
	let view = surface.root().unwrap();
	surface.dispatch_event(
		view,
		&NativeEvent::Scroll {
			offset_x: 0.0,
			offset_y: 200.0,
		},
	);
	surface.set_root(&rows(5, Some(asked(6, ScrollAlign::Nearest))));
	assert_eq!(scroll_offsets(&mut surface), []);

	surface.set_root(&rows(1, Some(asked(8, ScrollAlign::Nearest))));
	assert_eq!(
		scroll_offsets(&mut surface),
		[None, Some((0.0, 50.0, true))]
	);
}

#[test]
fn a_reveal_outside_a_scroll_view_is_dropped() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&View().child(
		Text("Saved.").with_props(|props| props.reveal = Some(asked(4, ScrollAlign::Center))),
	));

	assert_eq!(scroll_offsets(&mut surface), []);
}
