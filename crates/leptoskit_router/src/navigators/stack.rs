//! Stack navigator: push/pop with optional modal and sheet presentations.

use std::sync::Arc;

use leptoskit_native::Element;
use leptoskit_native::components::Pressable;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;

use crate::Render;
use crate::Routable;
use crate::Router;

/// How a stack presents a matched route.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Presentation {
	/// Ordinary push onto the stack.
	#[default]
	Push,
	/// Full-screen overlay above the previous route.
	Modal,
	/// Sheet-style overlay above the previous route.
	Sheet,
}

/// Native pop affordances the stack asks the host to honour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StackGesture {
	/// No extra chrome; Android system Back and `Router::back` still work.
	#[default]
	None,
	/// A leading edge hit target that pops when pressed. True interactive
	/// UIKit/AppKit pop needs host presentation and is not available yet.
	EdgeSwipe,
}

type PresentMatch<R> = Arc<dyn Fn(&R) -> bool + Send + Sync>;

/// A stack around a directory's screens.
#[derive(Clone)]
pub struct Stack<R> {
	gesture: StackGesture,
	presentations: Vec<(Presentation, PresentMatch<R>)>,
}

impl<R> std::fmt::Debug for Stack<R> {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Stack")
			.field("gesture", &self.gesture)
			.field("presentations", &self.presentations.len())
			.finish()
	}
}

impl<R> Stack<R> {
	/// A push stack with no special presentations.
	pub fn new() -> Self {
		Self {
			gesture: StackGesture::None,
			presentations: Vec::new(),
		}
	}
}

impl<R> Default for Stack<R> {
	fn default() -> Self {
		Self::new()
	}
}

impl<R> Stack<R>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	/// Sets the pop gesture affordance.
	#[must_use]
	pub fn gesture(mut self, gesture: StackGesture) -> Self {
		self.gesture = gesture;
		self
	}

	/// Presents routes matching `matches` as `presentation` instead of a push.
	#[must_use]
	pub fn present(
		mut self,
		presentation: Presentation,
		matches: impl Fn(&R) -> bool + Send + Sync + 'static,
	) -> Self {
		self.presentations.push((presentation, Arc::new(matches)));
		self
	}

	/// The presentation that applies to `route`, if any special one does.
	pub fn presentation_for(&self, route: &R) -> Presentation {
		self.presentations
			.iter()
			.find(|(_, matches)| matches(route))
			.map_or(Presentation::Push, |(presentation, _)| *presentation)
	}

	/// Wraps `children` with overlay chrome when the route is modal/sheet, and
	/// an edge hit target when [`StackGesture::EdgeSwipe`] is set.
	pub fn shell(&self, children: Element, navigation: &Router<R>, route: &R) -> Element
	where
		R: Render,
	{
		let presentation = self.presentation_for(route);
		let body = match presentation {
			Presentation::Push => children,
			Presentation::Modal | Presentation::Sheet => {
				overlay(children, navigation, presentation)
			}
		};

		match self.gesture {
			StackGesture::None => body,
			StackGesture::EdgeSwipe => edge_swipe(body, navigation),
		}
	}
}

fn overlay<R>(children: Element, navigation: &Router<R>, presentation: Presentation) -> Element
where
	R: Routable + Clone + PartialEq + Send + Sync + Render + 'static,
{
	// `shell` only calls this for Modal and Sheet; Push shares Modal chrome if
	// it ever reaches here so both arms stay reachable from tests.
	let (layer_class, layer_id) = match presentation {
		Presentation::Sheet => {
			(
				"absolute left-0 right-0 bottom-0 top-16 rounded-xl bg-white dark:bg-slate-950",
				"stack-sheet",
			)
		}
		Presentation::Modal | Presentation::Push => {
			("absolute inset-0 bg-white dark:bg-slate-950", "stack-modal")
		}
	};
	let layer = View().class(layer_class).test_id(layer_id).child(children);

	if let Some(previous) = navigation.previous() {
		let under = previous.render(navigation);
		View()
			.class("flex-1")
			.test_id("stack-overlay")
			.child(View().class("flex-1 opacity-40").child(under))
			.child(layer)
	} else {
		View().class("flex-1").test_id(layer_id).child(layer)
	}
}

fn edge_swipe<R>(children: Element, navigation: &Router<R>) -> Element
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	if !navigation.can_go_back() {
		return children;
	}

	let back = navigation.clone();
	View()
		.class("flex-1")
		.test_id("stack-edge-swipe")
		.child(children)
		.child(
			Pressable("Back")
				.class("absolute left-0 top-0 bottom-0 w-4")
				.test_id("stack-edge-hit")
				.on_press(move || {
					back.back();
				})
				.child(Text(String::new())),
		)
}
