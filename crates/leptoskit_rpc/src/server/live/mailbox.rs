//! One pending hub batch per connection. Repeated changes replace older
//! results without retaining a queue of database versions.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;

use tokio::sync::Notify;
use tokio::sync::mpsc::error::TryRecvError;

use super::hub::Batch;
use super::hub::Changed;
use crate::live::MAX_SUBSCRIPTIONS;

pub(super) struct Sender(Arc<Shared>);
pub(super) struct Receiver(Arc<Shared>);

#[derive(Default)]
struct Shared {
	state: Mutex<State>,
	ready: Notify,
}

#[derive(Default)]
struct State {
	pending: Option<Batch>,
	closed: bool,
}

pub(super) fn channel() -> (Sender, Receiver) {
	let shared = Arc::new(Shared::default());
	(Sender(Arc::clone(&shared)), Receiver(shared))
}

impl Shared {
	fn state(&self) -> MutexGuard<'_, State> {
		self.state.lock().unwrap_or_else(PoisonError::into_inner)
	}

	fn close(&self) {
		self.state().closed = true;
		self.ready.notify_one();
	}
}

impl Sender {
	/// Never waits for the connection. A batch includes at most one current
	/// result per registered subscription. Invalid oversized input closes
	/// the mailbox instead of discarding results and advancing its cursor.
	pub(super) fn send(&self, batch: Batch) -> bool {
		let mut state = self.0.state();
		if state.closed {
			return false;
		}
		let batch = match state.pending.take() {
			Some(previous) => coalesce(previous, batch),
			None => batch,
		};
		if batch.changes.len() > MAX_SUBSCRIPTIONS {
			state.closed = true;
		} else {
			state.pending = Some(batch);
		}
		let accepted = !state.closed;
		drop(state);
		self.0.ready.notify_one();
		accepted
	}

	/// A subscription ID may be reused. Its former result must not overwrite
	/// the replacement's initial value when the session consumes this batch.
	pub(super) fn remove(&self, id: u32) {
		if let Some(batch) = &mut self.0.state().pending {
			batch.changes.retain(|(member, _)| *member != id);
		}
	}
}

impl Receiver {
	pub(super) async fn recv(&mut self) -> Option<Batch> {
		loop {
			// notify_one retains a permit even before this future is polled.
			// There is exactly one receiver, including when select cancels it.
			let ready = self.0.ready.notified();
			match self.try_recv() {
				Ok(batch) => return Some(batch),
				Err(TryRecvError::Disconnected) => return None,
				Err(TryRecvError::Empty) => ready.await,
			}
		}
	}

	pub(super) fn try_recv(&self) -> Result<Batch, TryRecvError> {
		let mut state = self.0.state();
		if let Some(batch) = state.pending.take() {
			Ok(batch)
		} else if state.closed {
			Err(TryRecvError::Disconnected)
		} else {
			Err(TryRecvError::Empty)
		}
	}

	pub(super) fn is_empty(&self) -> bool {
		self.0.state().pending.is_none()
	}
}

impl Drop for Sender {
	fn drop(&mut self) {
		self.0.close();
	}
}

impl Drop for Receiver {
	fn drop(&mut self) {
		let mut state = self.0.state();
		state.closed = true;
		state.pending = None;
	}
}

/// The newer batch's patches were calculated against a result the session
/// might not have received. Complete values make the combined update
/// independent of intermediate versions, including error-to-success changes.
fn coalesce(previous: Batch, newer: Batch) -> Batch {
	let changes: BTreeMap<_, _> = previous
		.changes
		.into_iter()
		.chain(newer.changes)
		.map(|(id, changed)| {
			let changed = match changed {
				Changed::Patch(_, value) => Changed::Whole(value),
				other => other,
			};
			(id, changed)
		})
		.collect();
	Batch {
		to: previous.to.max(newer.to),
		full: previous.full || newer.full,
		revalidate: previous.revalidate || newer.revalidate,
		changes: changes.into_iter().collect(),
	}
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use serde_json::json;

	use super::*;
	use crate::live::Patch;
	use crate::live::WireError;

	fn batch(to: u64, changes: Vec<(u32, Changed)>) -> Batch {
		Batch {
			to,
			full: false,
			revalidate: false,
			changes,
		}
	}

	fn value(value: u64) -> Changed {
		Changed::Whole(Arc::new(json!(value)))
	}

	fn assert_value(batch: &Batch, id: u32, expected: u64) {
		let (_, Changed::Whole(value)) = batch.changes.iter().find(|(key, _)| *key == id).unwrap()
		else {
			panic!("coalesced results must contain complete values");
		};
		assert_eq!(**value, json!(expected));
	}

	#[test]
	fn a_slow_connection_retains_one_latest_result_per_subscription() {
		let (sender, receiver) = channel();
		for version in 1..=10_000 {
			assert!(sender.send(batch(version, vec![(1, value(version)), (2, value(7))])));
		}
		let current = receiver.try_recv().unwrap();
		assert_eq!(current.to, 10_000);
		assert_eq!(current.changes.len(), 2);
		assert_value(&current, 1, 10_000);
		assert_value(&current, 2, 7);
		assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
	}

	#[test]
	fn coalesced_patches_do_not_depend_on_unread_intermediate_values() {
		let (sender, receiver) = channel();
		let patch = |result| Changed::Patch(Arc::new(Patch::default()), Arc::new(json!(result)));
		assert!(sender.send(batch(1, vec![(1, patch(11)), (2, patch(22))])));
		assert!(sender.send(batch(2, vec![(1, patch(12))])));
		let current = receiver.try_recv().unwrap();
		assert_value(&current, 1, 12);
		assert_value(&current, 2, 22);
	}

	#[test]
	fn coalescing_releases_superseded_values() {
		let (sender, receiver) = channel();
		let old = Arc::new(json!(1));
		let weak = Arc::downgrade(&old);
		assert!(sender.send(batch(1, vec![(1, Changed::Whole(old))])));
		assert!(sender.send(batch(2, vec![(1, value(2))])));
		assert!(weak.upgrade().is_none());
		assert_value(&receiver.try_recv().unwrap(), 1, 2);
	}

	#[test]
	fn error_and_revalidation_survive_later_unrelated_changes() {
		let (sender, receiver) = channel();
		let error = WireError {
			status: 403,
			message: "policy changed".to_owned(),
		};
		let mut refused = batch(2, vec![(1, Changed::Failed(error.clone()))]);
		refused.revalidate = true;
		refused.full = true;
		assert!(sender.send(batch(1, vec![(1, value(1))])));
		assert!(sender.send(refused));
		assert!(sender.send(batch(3, vec![(2, value(3))])));
		let current = receiver.try_recv().unwrap();
		assert!(current.revalidate && current.full);
		assert!(matches!(&current.changes[0], (1, Changed::Failed(actual)) if *actual == error));
		assert_value(&current, 2, 3);
	}

	#[test]
	fn a_later_success_replaces_a_failure_without_reusing_its_old_value() {
		let (sender, receiver) = channel();
		let error = WireError {
			status: 503,
			message: "retry".to_owned(),
		};
		assert!(sender.send(batch(1, vec![(1, Changed::Failed(error))])));
		assert!(sender.send(batch(2, vec![(1, value(2))])));
		assert_value(&receiver.try_recv().unwrap(), 1, 2);
	}

	#[test]
	fn removing_and_reusing_ids_does_not_retain_old_values_or_grow_the_mailbox() {
		let (sender, receiver) = channel();
		for id in 1..=10_000 {
			assert!(sender.send(batch(u64::from(id), vec![(id, value(u64::from(id)))])));
			sender.remove(id);
		}
		assert!(receiver.try_recv().unwrap().changes.is_empty());
		assert!(sender.send(batch(10_001, vec![(1, value(1))])));
		sender.remove(1);
		assert!(sender.send(batch(10_002, vec![(1, value(2))])));
		assert_value(&receiver.try_recv().unwrap(), 1, 2);
	}

	#[tokio::test]
	async fn invalid_membership_closes_instead_of_advancing_past_discarded_results() {
		let (sender, mut receiver) = channel();
		let changes = (0..MAX_SUBSCRIPTIONS as u32)
			.map(|id| (id, value(1)))
			.collect();
		assert!(sender.send(batch(1, changes)));
		assert!(!sender.send(batch(2, vec![(MAX_SUBSCRIPTIONS as u32, value(2))])));
		assert!(receiver.recv().await.is_none());
		assert!(!sender.send(batch(3, vec![])));
	}

	#[tokio::test]
	async fn cancellation_does_not_consume_a_result_or_lose_the_next_wake() {
		let (sender, mut receiver) = channel();
		assert!(
			tokio::time::timeout(Duration::from_millis(1), receiver.recv())
				.await
				.is_err()
		);
		assert!(sender.send(batch(1, vec![(1, value(1))])));
		let current = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
			.await
			.unwrap()
			.unwrap();
		assert_value(&current, 1, 1);
		assert!(sender.send(batch(2, vec![(1, value(2))])));
		assert_value(&receiver.recv().await.unwrap(), 1, 2);
	}

	#[tokio::test]
	async fn sender_drop_delivers_pending_then_wakes_the_receiver_to_close() {
		let (sender, mut receiver) = channel();
		assert!(sender.send(batch(1, vec![(1, value(1))])));
		drop(sender);
		assert_value(&receiver.recv().await.unwrap(), 1, 1);
		assert!(receiver.recv().await.is_none());
	}

	#[test]
	fn receiver_drop_releases_values_and_rejects_further_batches() {
		let (sender, receiver) = channel();
		let old = Arc::new(json!(1));
		let weak = Arc::downgrade(&old);
		assert!(sender.send(batch(1, vec![(1, Changed::Whole(old))])));
		drop(receiver);
		assert!(weak.upgrade().is_none());
		assert!(!sender.send(batch(2, vec![])));
	}
}
