//! An unfinished transition keeps its remaining updates while the transport
//! is full. Only the final frame advances the complete resume cursor.

use std::collections::VecDeque;

use super::*;

pub(super) struct PendingTransition {
	from: u64,
	pub(super) to: u64,
	full: bool,
	updates: VecDeque<SubscriptionUpdate>,
}

impl PendingTransition {
	pub(super) fn new(from: u64, updates: Vec<SubscriptionUpdate>, to: u64, full: bool) -> Self {
		Self {
			from,
			to,
			full,
			updates: updates.into(),
		}
	}

	/// A replaced or removed subscription must not receive an older queued
	/// result.
	pub(super) fn remove(&mut self, id: u32) {
		self.updates.retain(|update| update.id != id);
	}

	/// Called only after a transport slot has been reserved. No frame is lost
	/// between constructing it and handing it to that slot.
	pub(super) fn frame(&mut self) -> Option<(Outbound, bool)> {
		let mut chunk = Vec::new();
		while let Some(update) = self.updates.pop_front() {
			chunk.push(update);
			let preview = ServerFrame::Transition(Transition {
				from: self.from,
				to: self.to,
				updates: chunk.clone(),
				full: self.full,
			});
			if serde_json::to_string(&preview).ok()?.len() > MAX_FRAME_BYTES && chunk.len() > 1 {
				self.updates.push_front(chunk.pop().expect("just pushed"));
				break;
			}
		}
		let complete = self.updates.is_empty();
		let to = if complete { self.to } else { self.from };
		let frame = Outbound::frame(
			&ServerFrame::Transition(Transition {
				from: self.from,
				to,
				updates: chunk,
				full: self.full,
			}),
			Some(to),
		)?;
		Some((frame, complete))
	}
}
