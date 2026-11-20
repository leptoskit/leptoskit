//! Phone-to-watch companion messaging and state sync (`WatchConnectivity` /
//! `WCSession` on watchOS and Wear OS Data Layer `MessageClient` /
//! `DataClient`).

use std::collections::BTreeMap;

use crate::WatchError;

/// Maximum UTF-8 bytes for a message topic.
pub const MAX_TOPIC_BYTES: usize = 64;
/// Maximum payload bytes for one companion transfer (64 KiB).
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;

/// Reachability and pairing snapshot for the companion phone app.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompanionState {
	/// Watch is paired with a companion phone.
	pub paired: bool,
	/// Companion phone is currently reachable for immediate messages.
	pub reachable: bool,
	/// The companion phone app is installed.
	pub companion_app_installed: bool,
}

/// Delivery semantics matching `WatchConnectivity` and Wear OS Data Layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeliveryMode {
	/// Send right now while reachable (`WCSession.sendMessage` /
	/// `MessageClient`).
	Immediate,
	/// Overwrite latest shared state dictionary (`updateApplicationContext` /
	/// `DataClient` path).
	ApplicationContext,
	/// Queued FIFO background delivery (`transferUserInfo`).
	BackgroundTransfer,
}

/// Bounded message exchanged between watch and companion phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionMessage {
	topic: String,
	payload: Vec<u8>,
	delivery: DeliveryMode,
}

impl CompanionMessage {
	/// Validates topic (`1..=64` ASCII id bytes) and payload (`<= 64 KiB`).
	///
	/// # Errors
	///
	/// Returns [`WatchError::InvalidId`] for bad topics or
	/// [`WatchError::InvalidState`] for oversized payloads.
	pub fn new(
		topic: impl Into<String>,
		payload: impl Into<Vec<u8>>,
		delivery: DeliveryMode,
	) -> Result<Self, WatchError> {
		let topic = topic.into();
		let payload = payload.into();
		if topic.is_empty()
			|| topic.len() > MAX_TOPIC_BYTES
			|| !topic
				.bytes()
				.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
		{
			return Err(WatchError::InvalidId(topic));
		}
		if payload.len() > MAX_PAYLOAD_BYTES {
			return Err(WatchError::InvalidState(format!(
				"companion payload is {} bytes; max is {MAX_PAYLOAD_BYTES}",
				payload.len()
			)));
		}
		Ok(Self {
			topic,
			payload,
			delivery,
		})
	}

	/// Delivery mode.
	#[must_use]
	pub const fn delivery(&self) -> DeliveryMode {
		self.delivery
	}

	/// Raw payload bytes.
	#[must_use]
	pub fn payload(&self) -> &[u8] {
		&self.payload
	}

	/// Message topic / path.
	#[must_use]
	pub fn topic(&self) -> &str {
		&self.topic
	}
}

/// Host contract for phone-watch companion communication.
pub trait CompanionLink {
	/// Current pairing and reachability state.
	fn state(&self) -> CompanionState;

	/// Sends or queues a message to the companion device.
	///
	/// # Errors
	///
	/// Returns [`WatchError::Unavailable`] when [`DeliveryMode::Immediate`] is
	/// requested while unreachable, or [`WatchError::InvalidState`] on failure.
	fn send(&mut self, message: CompanionMessage) -> Result<(), WatchError>;

	/// Drains the next incoming message from the companion device.
	fn poll_incoming(&mut self) -> Option<CompanionMessage>;
}

/// In-memory [`CompanionLink`] for deterministic tests and host stubs.
#[derive(Debug, Clone, Default)]
pub struct MemoryCompanionLink {
	state: CompanionState,
	outgoing: Vec<CompanionMessage>,
	application_context: BTreeMap<String, Vec<u8>>,
	incoming: Vec<CompanionMessage>,
}

impl MemoryCompanionLink {
	/// Starts a link with the given pairing/reachability snapshot.
	#[must_use]
	pub fn new(state: CompanionState) -> Self {
		Self {
			state,
			outgoing: Vec::new(),
			application_context: BTreeMap::new(),
			incoming: Vec::new(),
		}
	}

	/// Updates simulated reachability.
	pub fn set_state(&mut self, state: CompanionState) {
		self.state = state;
	}

	/// Simulates a message arriving from the phone.
	pub fn push_incoming(&mut self, message: CompanionMessage) {
		self.incoming.push(message);
	}

	/// Latest application context payload stored for `topic`.
	#[must_use]
	pub fn application_context(&self, topic: &str) -> Option<&[u8]> {
		self.application_context.get(topic).map(Vec::as_slice)
	}

	/// Outgoing messages sent so far.
	#[must_use]
	pub fn outgoing(&self) -> &[CompanionMessage] {
		&self.outgoing
	}
}

impl CompanionLink for MemoryCompanionLink {
	fn state(&self) -> CompanionState {
		self.state
	}

	fn send(&mut self, message: CompanionMessage) -> Result<(), WatchError> {
		if !self.state.paired {
			return Err(WatchError::Unavailable(
				"no companion phone is paired".into(),
			));
		}
		if message.delivery == DeliveryMode::Immediate && !self.state.reachable {
			return Err(WatchError::Unavailable(
				"companion phone is not currently reachable for immediate delivery".into(),
			));
		}
		if message.delivery == DeliveryMode::ApplicationContext {
			self.application_context
				.insert(message.topic.clone(), message.payload.clone());
		}
		self.outgoing.push(message);
		Ok(())
	}

	fn poll_incoming(&mut self) -> Option<CompanionMessage> {
		if self.incoming.is_empty() {
			None
		} else {
			Some(self.incoming.remove(0))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn companion_link_enforces_reachability_and_updates_context() {
		let mut link = MemoryCompanionLink::new(CompanionState {
			paired: true,
			reachable: false,
			companion_app_installed: true,
		});

		let immediate =
			CompanionMessage::new("workout/ping", b"hi".to_vec(), DeliveryMode::Immediate).unwrap();
		assert!(link.send(immediate).is_err());

		// ApplicationContext succeeds even when temporarily unreachable.
		let ctx = CompanionMessage::new(
			"user/goal",
			b"10000".to_vec(),
			DeliveryMode::ApplicationContext,
		)
		.unwrap();
		link.send(ctx).unwrap();
		assert_eq!(
			link.application_context("user/goal"),
			Some(b"10000".as_slice())
		);

		// Once reachable, immediate delivery succeeds.
		link.set_state(CompanionState {
			paired: true,
			reachable: true,
			companion_app_installed: true,
		});
		let live =
			CompanionMessage::new("workout/ping", b"hi".to_vec(), DeliveryMode::Immediate).unwrap();
		link.send(live).unwrap();
		assert_eq!(link.outgoing().len(), 2);
	}
}
