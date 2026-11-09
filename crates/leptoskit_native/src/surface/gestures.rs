//! Routes pointer samples through the shared arena.

use leptoskit_yoga::NodeId;

use super::NativeSurface;
use crate::backend::NativeBackend;
use crate::gesture::Collected;
use crate::gesture::Effect;
use crate::gesture::PointerDecision;
use crate::gesture::PointerPhase;
use crate::gesture::PointerSample;
use crate::props::NativeKind;

impl<B: NativeBackend> NativeSurface<B> {
	/// Feeds one pointer sample to the gesture arena and runs the callbacks
	/// of any recognizer that moved.
	///
	/// Coordinates are viewport layout units, the same space as
	/// [`hit_test`](Self::hit_test). The returned route tells the platform
	/// whether to keep handling the pointer itself. Callbacks run before this
	/// returns, after the arena has released its borrow, so a callback may
	/// rebuild the tree. A callback must not feed another sample for the same
	/// pointer; the platform sends the next sample after this one returns.
	pub fn pointer(&self, sample: PointerSample) -> PointerDecision {
		let collected = self.gesture_targets(&sample);
		let (decision, effects) = {
			let mut session = self.gestures.borrow_mut();
			let decision = session.handle(sample, collected);
			let effects = session.take_effects();
			(decision, effects)
		};
		self.fire_gestures(effects);
		decision
	}

	/// Advances recognizers that are waiting on the clock, such as a long
	/// press held still. `now_ms` uses the same clock as the samples.
	pub fn advance_gestures(&self, now_ms: u64) -> PointerDecision {
		let (decision, effects) = {
			let mut session = self.gestures.borrow_mut();
			let decision = session.advance(now_ms);
			let effects = session.take_effects();
			(decision, effects)
		};
		self.fire_gestures(effects);
		decision
	}

	pub(super) fn fire_gestures(&self, effects: Vec<Effect>) {
		for effect in &effects {
			(effect.callback)(effect.event);
		}

		self.gestures.borrow_mut().recycle(effects);
	}

	fn gesture_targets(&self, sample: &PointerSample) -> Vec<Collected> {
		if sample.phase != PointerPhase::Down {
			return Vec::new();
		}

		let Some(target) = self.hit_test(sample.x, sample.y) else {
			return Vec::new();
		};

		if !self.event_path_is_active(target) {
			return Vec::new();
		}

		let mut targets = Vec::new();
		let mut id = Some(target);

		while let Some(node) = id {
			push_target(self, node, &mut targets);
			id = self.shadows[node.index()].parent;
		}

		if targets.iter().all(|target| target.gesture.is_none()) {
			targets.clear();
		}

		targets
	}
}

fn push_target<B: NativeBackend>(
	surface: &NativeSurface<B>,
	node: NodeId,
	targets: &mut Vec<Collected>,
) {
	let shadow = &surface.shadows[node.index()];
	let native = native_participant(shadow.kind);

	if shadow.gesture.is_none() && native.is_none() {
		return;
	}

	targets.push(Collected {
		node,
		gesture: shadow.gesture.clone(),
		native,
	});
}

/// `Some(true)` when the control wants a drag. `Some(false)` when it wants a
/// click or a focus. `None` when the arena should not invent a participant.
fn native_participant(kind: NativeKind) -> Option<bool> {
	match kind {
		NativeKind::ScrollView | NativeKind::Slider => Some(true),
		NativeKind::Button
		| NativeKind::Pressable
		| NativeKind::TextInput
		| NativeKind::PasswordInput
		| NativeKind::Switch
		| NativeKind::Checkbox
		| NativeKind::Radio => Some(false),
		_ => None,
	}
}
