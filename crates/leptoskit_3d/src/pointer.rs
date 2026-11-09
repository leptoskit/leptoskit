//! Pointer input from a native scene's view, independent of asset events.

use crate::DecodeError;

/// One primary finger or mouse pointer's phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PointerPhase {
	/// The pointer went down.
	Down = 0,
	/// The pointer moved beyond the platform's drag slop.
	Move = 1,
	/// The pointer came up normally.
	Up = 2,
	/// The gesture was interrupted; release held controls without tapping.
	Cancel = 3,
}

impl PointerPhase {
	/// Decodes the native ABI tag.
	pub fn from_tag(tag: u32) -> Option<Self> {
		[Self::Down, Self::Move, Self::Up, Self::Cancel]
			.get(tag as usize)
			.copied()
	}
}

/// Input on one scene view. Positions are fractions across and down the
/// view; deltas are layout units (Android dp, Apple points), never pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerInput {
	/// What happened.
	pub phase: PointerPhase,
	/// Across the view, from 0 to 1.
	pub x: f32,
	/// Down the view, from 0 to 1.
	pub y: f32,
	/// Movement to the right in layout units.
	pub dx: f32,
	/// Movement down in layout units.
	pub dy: f32,
}

impl PointerInput {
	/// Bytes in one input reply. An empty reply means the scene closed.
	pub const WIRE_SIZE: usize = 20;

	/// Whether the input is finite and within the native input contract.
	pub fn is_valid(self) -> bool {
		(0.0..=1.0).contains(&self.x)
			&& (0.0..=1.0).contains(&self.y)
			&& self.dx.is_finite()
			&& self.dy.is_finite()
			&& self.dx.abs() <= 1_000_000.0
			&& self.dy.abs() <= 1_000_000.0
	}

	/// The compact native reply.
	pub fn encode(self) -> Vec<u8> {
		[
			self.phase as u32,
			self.x.to_bits(),
			self.y.to_bits(),
			self.dx.to_bits(),
			self.dy.to_bits(),
		]
		.into_iter()
		.flat_map(u32::to_le_bytes)
		.collect()
	}

	/// Decodes one input, rejecting unknown tags, non-finite values and
	/// trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
		if bytes.len() != Self::WIRE_SIZE {
			return Err(DecodeError("invalid pointer input length"));
		}
		let words: [u32; 5] = std::array::from_fn(|index| {
			u32::from_le_bytes(
				bytes[index * 4..index * 4 + 4]
					.try_into()
					.expect("four bytes"),
			)
		});
		let input = Self {
			phase: PointerPhase::from_tag(words[0]).ok_or(DecodeError("unknown pointer phase"))?,
			x: f32::from_bits(words[1]),
			y: f32::from_bits(words[2]),
			dx: f32::from_bits(words[3]),
			dy: f32::from_bits(words[4]),
		};
		if !input.is_valid() {
			return Err(DecodeError("invalid pointer coordinates"));
		}
		Ok(input)
	}
}

#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub(crate) mod host {
	use std::collections::HashMap;
	use std::collections::VecDeque;
	use std::sync::Arc;
	use std::sync::Mutex;
	use std::sync::Weak;
	use std::task::Poll;
	use std::task::Waker;

	use super::PointerInput;
	use super::PointerPhase;
	use crate::SceneId;

	#[derive(Default)]
	struct Queue {
		inputs: VecDeque<PointerInput>,
		waiters: Vec<Weak<Waker>>,
	}

	/// Bounded input queues, separate from model-loading and animation
	/// events so independent subscribers cannot steal each other's data.
	#[derive(Default)]
	pub struct Inputs(Mutex<HashMap<SceneId, Queue>>);

	impl Inputs {
		pub fn create(&self, scene: SceneId) {
			self.0
				.lock()
				.unwrap_or_else(std::sync::PoisonError::into_inner)
				.entry(scene)
				.or_default();
		}

		pub fn close(&self, scene: SceneId) {
			let queue = self
				.0
				.lock()
				.unwrap_or_else(std::sync::PoisonError::into_inner)
				.remove(&scene);
			if let Some(queue) = queue {
				for waiter in queue.waiters {
					if let Some(waiter) = waiter.upgrade() {
						waiter.wake_by_ref();
					}
				}
			}
		}

		pub fn push(&self, scene: SceneId, input: PointerInput) -> bool {
			if !input.is_valid() {
				return false;
			}
			let waiters = {
				let mut queues = self
					.0
					.lock()
					.unwrap_or_else(std::sync::PoisonError::into_inner);
				let Some(queue) = queues.get_mut(&scene) else {
					return false;
				};
				if input.phase == PointerPhase::Move
					&& let Some(last) = queue.inputs.back_mut()
					&& last.phase == PointerPhase::Move
				{
					last.x = input.x;
					last.y = input.y;
					last.dx = (last.dx + input.dx).clamp(-1_000_000.0, 1_000_000.0);
					last.dy = (last.dy + input.dy).clamp(-1_000_000.0, 1_000_000.0);
				} else {
					if queue.inputs.len() >= 64 {
						queue.inputs.clear();
						queue.inputs.push_back(PointerInput {
							phase: PointerPhase::Cancel,
							dx: 0.0,
							dy: 0.0,
							..input
						});
					}
					queue.inputs.push_back(input);
				}
				std::mem::take(&mut queue.waiters)
			};
			for waiter in waiters {
				if let Some(waiter) = waiter.upgrade() {
					waiter.wake_by_ref();
				}
			}
			true
		}

		pub fn poll(&self, scene: SceneId, waker: &Arc<Waker>) -> Poll<Option<PointerInput>> {
			let mut queues = self
				.0
				.lock()
				.unwrap_or_else(std::sync::PoisonError::into_inner);
			let Some(queue) = queues.get_mut(&scene) else {
				return Poll::Ready(None);
			};
			if let Some(input) = queue.inputs.pop_front() {
				return Poll::Ready(Some(input));
			}
			// A cancelled future drops its strong waker. Prune it on the
			// next subscription, even if no pointer input ever arrives.
			queue.waiters.retain(|waiter| waiter.strong_count() > 0);
			let weak = Arc::downgrade(waker);
			if !queue.waiters.iter().any(|waiter| waiter.ptr_eq(&weak)) {
				queue.waiters.push(weak);
			}
			Poll::Pending
		}
	}
	#[cfg(test)]
	#[test]
	fn cancelled_subscriptions_do_not_accumulate_wakers() {
		let inputs = Inputs::default();
		let scene = SceneId(9);
		inputs.create(scene);
		for _ in 0..1000 {
			let waker = Arc::new(Waker::noop().clone());
			assert!(inputs.poll(scene, &waker).is_pending());
		}
		let queues = inputs.0.lock().unwrap();
		assert_eq!(queues[&scene].waiters.len(), 1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn input(phase: PointerPhase) -> PointerInput {
		PointerInput {
			phase,
			x: 0.3,
			y: 0.6,
			dx: 12.0,
			dy: -4.0,
		}
	}

	#[test]
	fn input_wire_roundtrips_and_rejects_invalid_values() {
		for phase in [
			PointerPhase::Down,
			PointerPhase::Move,
			PointerPhase::Up,
			PointerPhase::Cancel,
		] {
			let input = input(phase);
			assert_eq!(input.encode().len(), PointerInput::WIRE_SIZE);
			assert_eq!(PointerInput::decode(&input.encode()), Ok(input));
		}
		assert!(PointerInput::decode(&[]).is_err());
		let mut bytes = input(PointerPhase::Move).encode();
		bytes[0] = 4;
		assert!(PointerInput::decode(&bytes).is_err());
		for invalid in [
			PointerInput {
				x: f32::NAN,
				..input(PointerPhase::Down)
			},
			PointerInput {
				y: 1.1,
				..input(PointerPhase::Down)
			},
			PointerInput {
				dx: f32::INFINITY,
				..input(PointerPhase::Move)
			},
		] {
			assert!(PointerInput::decode(&invalid.encode()).is_err());
		}
		let mut bytes = input(PointerPhase::Down).encode();
		bytes.push(0);
		assert!(PointerInput::decode(&bytes).is_err());
	}

	#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
	#[test]
	fn slow_consumers_keep_drag_distance_and_release_after_overflow() {
		use std::sync::Arc;
		use std::task::Poll;
		use std::task::Waker;
		let inputs = host::Inputs::default();
		let scene = crate::SceneId(1);
		inputs.create(scene);
		let waker = Arc::new(Waker::noop().clone());
		inputs.push(scene, input(PointerPhase::Down));
		for _ in 0..100 {
			inputs.push(scene, input(PointerPhase::Move));
		}
		inputs.push(scene, input(PointerPhase::Up));
		assert_eq!(
			inputs.poll(scene, &waker),
			Poll::Ready(Some(input(PointerPhase::Down)))
		);
		assert_eq!(
			inputs.poll(scene, &waker),
			Poll::Ready(Some(PointerInput {
				dx: 1200.0,
				dy: -400.0,
				..input(PointerPhase::Move)
			}))
		);
		assert_eq!(
			inputs.poll(scene, &waker),
			Poll::Ready(Some(input(PointerPhase::Up)))
		);
		assert!(inputs.poll(scene, &waker).is_pending());
		for _ in 0..33 {
			inputs.push(scene, input(PointerPhase::Down));
			inputs.push(scene, input(PointerPhase::Up));
		}
		assert!(matches!(
			inputs.poll(scene, &waker),
			Poll::Ready(Some(PointerInput {
				phase: PointerPhase::Cancel,
				..
			}))
		));
		inputs.close(scene);
		assert_eq!(inputs.poll(scene, &waker), Poll::Ready(None));
		assert!(!inputs.push(scene, input(PointerPhase::Down)));
	}
}
