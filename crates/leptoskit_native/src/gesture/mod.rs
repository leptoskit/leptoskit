//! Pointer gestures for the shared widget tree.
//!
//! A screen attaches recognizers with
//! [`Element::gesture`](crate::Element::gesture). The surface hit-tests each
//! pointer and runs one arena, the way Flutter does: recognizers along the hit
//! path compete until one accepts, and the others fail. Composition follows
//! react-native-gesture-handler: [`Gesture::simultaneous`] lets recognizers
//! proceed together (pinch and rotation), [`Gesture::exclusive`] gives the
//! first gesture priority, and [`Gesture::race`] gives the gesture to whichever
//! accepts first.
//!
//! Android and Linux deliver the same [`PointerSample`] values into
//! [`NativeSurface::pointer`](crate::NativeSurface::pointer). Recognizers that
//! win a drag claim the pointer so the platform control does not also click
//! or scroll. A tap leaves the pointer with the platform, so a `Button` still
//! receives its press. A pointer that is cancelled, lifted, or whose node
//! leaves the tree always ends; a later sample cannot find a gesture that is
//! still down.

mod engine;

use std::sync::Arc;
use std::time::Duration;

pub(crate) use engine::Collected;
pub(crate) use engine::Effect;
pub(crate) use engine::Session;

/// Which part of a gesture a callback is seeing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureState {
	/// The recognizer won the arena and the gesture has started.
	Began,
	/// The pointer moved while the gesture was active.
	Active,
	/// The pointer lifted and the gesture completed.
	End,
	/// The pointer was cancelled, or the node left the tree, after the
	/// gesture had started.
	Cancelled,
}

/// Where a pointer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
	/// A finger.
	Touch,
	/// A mouse or trackpad click.
	Mouse,
	/// A stylus.
	Pen,
}

/// What the platform should do with the pointer that was just reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerRoute {
	/// No gesture wants the pointer. The platform control handles it.
	Platform,
	/// A gesture is still deciding. Keep delivering samples, and let the
	/// platform control see them too.
	Track,
	/// A gesture owns the pointer. Cancel the platform control and keep the
	/// rest of the samples for the recognizer.
	Claim,
}

/// One sample in a pointer's life, in viewport layout units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerSample {
	/// Stable id for this contact. A mouse uses one id; each finger uses its
	/// own.
	pub id: u64,
	/// Whether the contact went down, moved, came up, or was cancelled.
	pub phase: PointerPhase,
	/// Viewport x, in layout units.
	pub x: f32,
	/// Viewport y, in layout units.
	pub y: f32,
	/// Platform time in milliseconds. The clock only has to be monotonic.
	pub time_ms: u64,
	/// The device that produced the sample.
	pub kind: PointerKind,
}

/// Down, move, up, or cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase {
	/// The contact touched the screen.
	Down,
	/// The contact moved.
	Move,
	/// The contact lifted.
	Up,
	/// The platform took the contact away.
	Cancel,
}

impl PointerSample {
	/// A touch sample.
	#[must_use]
	pub const fn new(id: u64, phase: PointerPhase, x: f32, y: f32, time_ms: u64) -> Self {
		Self {
			id,
			phase,
			x,
			y,
			time_ms,
			kind: PointerKind::Touch,
		}
	}

	/// The same sample from a mouse or a pen.
	#[must_use]
	pub const fn kind(mut self, kind: PointerKind) -> Self {
		self.kind = kind;
		self
	}
}

/// What the arena decided, and when a stationary pointer should be advanced.
///
/// `deadline_ms` is an absolute time on the same clock as
/// [`PointerSample::time_ms`]. The host calls
/// [`NativeSurface::advance_gestures`](crate::NativeSurface::advance_gestures)
/// at that time so a long press can win without another move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerDecision {
	/// How the host should treat this pointer.
	pub route: PointerRoute,
	/// When to advance the arena, if a recognizer is waiting on the clock.
	pub deadline_ms: Option<u64>,
}

impl PointerDecision {
	/// The platform owns the pointer and nothing is waiting on the clock.
	#[must_use]
	pub const fn platform() -> Self {
		Self {
			route: PointerRoute::Platform,
			deadline_ms: None,
		}
	}
}

/// A gesture callback's view of one moment.
///
/// Distances are in layout units. Velocity is layout units per second.
/// [`scale`](Self::scale) starts at 1 and [`rotation`](Self::rotation) is in
/// radians, clockwise positive in screen space (y grows downward). Fields
/// that a recognizer does not use stay at their neutral value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GestureEvent {
	/// Which callback this is.
	pub state: GestureState,
	/// Latest viewport x. For a pinch or rotation this is the focal point.
	pub x: f32,
	/// Latest viewport y.
	pub y: f32,
	/// Movement from where the gesture began.
	pub translation_x: f32,
	/// Movement from where the gesture began.
	pub translation_y: f32,
	/// Horizontal velocity in layout units per second.
	pub velocity_x: f32,
	/// Vertical velocity in layout units per second.
	pub velocity_y: f32,
	/// Distance between two pointers, divided by that distance when the
	/// gesture began.
	pub scale: f32,
	/// Rotation since the gesture began, in radians.
	pub rotation: f32,
	/// How many pointers the gesture is using.
	pub pointers: u8,
	/// The pointer that started the gesture.
	pub pointer: u64,
	/// The device that produced the pointer.
	pub kind: PointerKind,
}

/// A recognizer, or a composition of recognizers, attached to one element.
#[derive(Clone)]
pub struct Gesture {
	node: GestureNode,
}

#[derive(Clone)]
enum GestureNode {
	Tap {
		slop: f32,
		max_ms: Option<u64>,
		calls: Calls,
	},
	LongPress {
		slop: f32,
		duration_ms: u64,
		calls: Calls,
	},
	Pan {
		slop: f32,
		axis: Axis,
		preempt: bool,
		calls: Calls,
	},
	Pinch {
		calls: Calls,
	},
	Rotation {
		calls: Calls,
	},
	Fling {
		min_speed: f32,
		calls: Calls,
	},
	Native,
	Group {
		mode: GroupMode,
		children: Vec<Gesture>,
	},
}

#[derive(Clone, Copy)]
enum Axis {
	Free,
	Horizontal,
	Vertical,
}

#[derive(Clone, Copy)]
enum GroupMode {
	Simultaneous,
	Exclusive,
	Race,
}

#[derive(Clone, Default)]
struct Calls {
	begin: Option<Callback>,
	update: Option<Callback>,
	end: Option<Callback>,
	cancel: Option<Callback>,
}

type Callback = Arc<dyn Fn(GestureEvent) + Send + Sync>;

impl std::fmt::Debug for Gesture {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self.node {
			GestureNode::Tap { slop, max_ms, .. } => {
				f.debug_struct("Tap")
					.field("slop", slop)
					.field("max_ms", max_ms)
					.finish()
			}
			GestureNode::LongPress {
				slop, duration_ms, ..
			} => {
				f.debug_struct("LongPress")
					.field("slop", slop)
					.field("duration_ms", duration_ms)
					.finish()
			}
			GestureNode::Pan {
				slop,
				axis,
				preempt,
				..
			} => {
				f.debug_struct("Pan")
					.field("slop", slop)
					.field("axis", axis)
					.field("preempt", preempt)
					.finish()
			}
			GestureNode::Pinch { .. } => f.write_str("Pinch"),
			GestureNode::Rotation { .. } => f.write_str("Rotation"),
			GestureNode::Fling { min_speed, .. } => {
				f.debug_struct("Fling")
					.field("min_speed", min_speed)
					.finish()
			}
			GestureNode::Native => f.write_str("Native"),
			GestureNode::Group { mode, children } => {
				f.debug_struct("Group")
					.field("mode", mode)
					.field("children", children)
					.finish()
			}
		}
	}
}

impl std::fmt::Debug for Axis {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			Self::Free => "free",
			Self::Horizontal => "horizontal",
			Self::Vertical => "vertical",
		})
	}
}

impl std::fmt::Debug for GroupMode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			Self::Simultaneous => "simultaneous",
			Self::Exclusive => "exclusive",
			Self::Race => "race",
		})
	}
}

impl Gesture {
	/// A tap that completes when the pointer lifts inside the touch slop.
	///
	/// The slop is 18 layout units, Flutter's touch slop. There is no time
	/// limit unless [`max_duration`](Self::max_duration) sets one, so a long
	/// hold still taps when no long-press recognizer wins the arena first.
	#[must_use]
	pub fn tap() -> Self {
		Self {
			node: GestureNode::Tap {
				slop: engine::TOUCH_SLOP,
				max_ms: None,
				calls: Calls::default(),
			},
		}
	}

	/// A press that wins after it has been held still.
	///
	/// The default is half a second, inside the touch slop. Moving farther
	/// than that before the deadline makes the long press fail so a pan can
	/// win.
	#[must_use]
	pub fn long_press() -> Self {
		Self {
			node: GestureNode::LongPress {
				slop: engine::TOUCH_SLOP,
				duration_ms: engine::LONG_PRESS_MS,
				calls: Calls::default(),
			},
		}
	}

	/// A drag that wins once the pointer travels past the touch slop.
	#[must_use]
	pub fn pan() -> Self {
		Self {
			node: GestureNode::Pan {
				slop: engine::TOUCH_SLOP,
				axis: Axis::Free,
				preempt: false,
				calls: Calls::default(),
			},
		}
	}

	/// A two-pointer pinch. Scale is the distance between the pointers
	/// divided by the distance when the second pointer landed.
	#[must_use]
	pub fn pinch() -> Self {
		Self {
			node: GestureNode::Pinch {
				calls: Calls::default(),
			},
		}
	}

	/// A two-pointer rotation, in radians, from the line between the pointers
	/// when the second pointer landed.
	#[must_use]
	pub fn rotation() -> Self {
		Self {
			node: GestureNode::Rotation {
				calls: Calls::default(),
			},
		}
	}

	/// A fling that wins on lift when the pointer is moving faster than a
	/// threshold. The default is 800 layout units per second.
	#[must_use]
	pub fn fling() -> Self {
		Self {
			node: GestureNode::Fling {
				min_speed: engine::FLING_SPEED,
				calls: Calls::default(),
			},
		}
	}

	/// The platform control under this element: a button's click, a scroll,
	/// or a slider.
	///
	/// The arena inserts one of these on its own for a control that sits on
	/// a gesture's hit path. An explicit native gesture is how a composition
	/// names that control, for example a pan that must lose to the scroll
	/// unless it is marked [`blocks_native`](Self::blocks_native).
	#[must_use]
	pub fn native() -> Self {
		Self {
			node: GestureNode::Native,
		}
	}

	/// Both gestures can win. Neither failing the other is how a pinch and a
	/// rotation share one two-finger gesture.
	#[must_use]
	pub fn simultaneous(self, other: Self) -> Self {
		self.group(GroupMode::Simultaneous, other)
	}

	/// `self` has priority. `other` can win only after `self` fails, which is
	/// react-native-gesture-handler's `Exclusive`.
	#[must_use]
	pub fn exclusive(self, other: Self) -> Self {
		self.group(GroupMode::Exclusive, other)
	}

	/// Whichever gesture accepts first wins and the other fails. This is the
	/// arena's ordinary rule, named so a race can sit inside another group.
	#[must_use]
	pub fn race(self, other: Self) -> Self {
		self.group(GroupMode::Race, other)
	}

	/// Called when the recognizer wins the arena.
	#[must_use]
	pub fn on_begin(mut self, callback: impl Fn(GestureEvent) + Send + Sync + 'static) -> Self {
		if let Some(calls) = self.calls_mut() {
			calls.begin = Some(Arc::new(callback));
		}

		self
	}

	/// Called when an active gesture's pointers move.
	#[must_use]
	pub fn on_update(mut self, callback: impl Fn(GestureEvent) + Send + Sync + 'static) -> Self {
		if let Some(calls) = self.calls_mut() {
			calls.update = Some(Arc::new(callback));
		}

		self
	}

	/// Called when the pointer lifts and the gesture completes.
	#[must_use]
	pub fn on_end(mut self, callback: impl Fn(GestureEvent) + Send + Sync + 'static) -> Self {
		if let Some(calls) = self.calls_mut() {
			calls.end = Some(Arc::new(callback));
		}

		self
	}

	/// Called when an active gesture is cancelled.
	#[must_use]
	pub fn on_cancel(mut self, callback: impl Fn(GestureEvent) + Send + Sync + 'static) -> Self {
		if let Some(calls) = self.calls_mut() {
			calls.cancel = Some(Arc::new(callback));
		}

		self
	}

	/// How far a tap or a long press may drift, in layout units, and still win.
	/// Also the distance a pan must travel.
	#[must_use]
	pub fn max_distance(mut self, distance: f32) -> Self {
		let distance = if distance.is_finite() {
			distance.max(0.0)
		} else {
			0.0
		};

		match &mut self.node {
			GestureNode::Tap { slop, .. }
			| GestureNode::LongPress { slop, .. }
			| GestureNode::Pan { slop, .. } => *slop = distance,
			_ => {}
		}

		self
	}

	/// The longest a tap may last. After this the tap fails on the next sample
	/// or when the host advances the arena.
	#[must_use]
	pub fn max_duration(mut self, duration: Duration) -> Self {
		if let GestureNode::Tap { max_ms, .. } = &mut self.node {
			*max_ms = Some(millis(duration));
		}

		self
	}

	/// How long a finger must hold still before a long press wins.
	#[must_use]
	pub fn min_duration(mut self, duration: Duration) -> Self {
		if let GestureNode::LongPress { duration_ms, .. } = &mut self.node {
			*duration_ms = millis(duration);
		}

		self
	}

	/// The slowest lift, in layout units per second, that still counts as a
	/// fling.
	#[must_use]
	pub fn min_velocity(mut self, speed: f32) -> Self {
		if let GestureNode::Fling { min_speed, .. } = &mut self.node {
			*min_speed = if speed.is_finite() {
				speed.max(0.0)
			} else {
				0.0
			};
		}

		self
	}

	/// A pan that only wins for a mostly horizontal drag. A vertical drag
	/// fails it so a parent or a scroll can take the pointer.
	#[must_use]
	pub fn horizontal(mut self) -> Self {
		if let GestureNode::Pan { axis, .. } = &mut self.node {
			*axis = Axis::Horizontal;
		}

		self
	}

	/// A pan that only wins for a mostly vertical drag.
	#[must_use]
	pub fn vertical(mut self) -> Self {
		if let GestureNode::Pan { axis, .. } = &mut self.node {
			*axis = Axis::Vertical;
		}

		self
	}

	/// This pan wins even when a scroll view or a slider on the hit path
	/// would otherwise take the drag.
	///
	/// Without this, the control closer to the pointer wins a drag, which is
	/// the same rule as the rest of the arena: the deepest recognizer that
	/// accepts is first.
	#[must_use]
	pub fn blocks_native(mut self) -> Self {
		if let GestureNode::Pan { preempt, .. } = &mut self.node {
			*preempt = true;
		}

		self
	}

	fn group(self, mode: GroupMode, other: Self) -> Self {
		let children = match self.node {
			GestureNode::Group {
				mode: existing,
				children,
			} if std::mem::discriminant(&existing) == std::mem::discriminant(&mode) => {
				let mut children = children;
				children.push(other);
				children
			}
			_ => vec![self, other],
		};

		Self {
			node: GestureNode::Group { mode, children },
		}
	}

	fn calls_mut(&mut self) -> Option<&mut Calls> {
		match &mut self.node {
			GestureNode::Tap { calls, .. }
			| GestureNode::LongPress { calls, .. }
			| GestureNode::Pan { calls, .. }
			| GestureNode::Pinch { calls }
			| GestureNode::Rotation { calls }
			| GestureNode::Fling { calls, .. } => Some(calls),
			GestureNode::Native | GestureNode::Group { .. } => None,
		}
	}
}

fn millis(duration: Duration) -> u64 {
	u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
