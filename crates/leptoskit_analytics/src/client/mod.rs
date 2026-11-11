//! The client: what an app records events with, the same on the web, in a
//! native app, and on a server.
//!
//! [`Client`] keeps who the person is, their session, and the properties
//! every event gets, and queues what's recorded in a [`Queue`]. It does no
//! I/O: a [`Platform`] gives it the time and random bytes, and the
//! platform's transport takes batches from [`Client::queue`] and sends
//! them. The transports are `server` (this crate's `server-client`
//! feature) and, in later releases, the browser and native apps.
//!
//! It depends on nothing but the standard library, so an app that records
//! events compiles no JSON, UUID, time, or compression crate for it.

pub mod queue;

#[cfg(feature = "server-client")]
pub mod server;

pub use self::queue::Limits;
pub use self::queue::Outcome;
pub use self::queue::Pending;
pub use self::queue::Queue;
use crate::wire;
use crate::wire::Body;
use crate::wire::Entry;
use crate::wire::Properties;
use crate::wire::Value;

/// What a client needs from where it runs.
pub trait Platform {
	/// The time, in milliseconds since the Unix epoch.
	fn now(&self) -> i64;

	/// Fills `bytes` with random bytes, for ids. They needn't be
	/// cryptographically strong; they must not repeat.
	fn fill_random(&self, bytes: &mut [u8]);
}

/// How a client batches and when its sessions end.
#[derive(Clone, Debug)]
pub struct Config {
	pub limits: Limits,
	/// A session ends after this long without an event, in milliseconds.
	pub session_idle: i64,
	/// A session ends after this long, in milliseconds, however busy.
	pub session_max: i64,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			limits: Limits::default(),
			session_idle: 30 * 60 * 1_000,
			session_max: 24 * 60 * 60 * 1_000,
		}
	}
}

/// Who a client records events as: what a platform stores so the same
/// person is recognised next time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
	pub distinct_id: String,
	/// Whether `distinct_id` is the app's id for a signed-in person, rather
	/// than one the client made up.
	pub identified: bool,
}

struct Session {
	id: [u8; 16],
	started: i64,
	last: i64,
}

/// Records events for one person at a time, as a browser or an app on a
/// device does.
pub struct Client<P> {
	platform: P,
	config: Config,
	queue: Queue,
	identity: Identity,
	session: Option<Session>,
	/// Added to every event: [`register`](Self::register)'s properties.
	registered: Properties,
	/// The groups every event is about, as `$groups`.
	groups: Properties,
	opted_out: bool,
}

impl<P: Platform> Client<P> {
	/// A client for someone new, with a made-up id, whose batches carry
	/// `context`: the library, the device, the app's version.
	pub fn new(platform: P, config: Config, context: Properties) -> Self {
		let identity = Identity {
			distinct_id: anonymous_id(&platform),
			identified: false,
		};
		Self::restore(platform, config, context, identity)
	}

	/// A client for someone it has seen before, as `identity`.
	pub fn restore(platform: P, config: Config, context: Properties, identity: Identity) -> Self {
		let queue = Queue::new(config.limits.clone(), context);
		Self {
			platform,
			config,
			queue,
			identity,
			session: None,
			registered: Vec::new(),
			groups: Vec::new(),
			opted_out: false,
		}
	}

	/// Who events are recorded as.
	pub fn identity(&self) -> &Identity {
		&self.identity
	}

	/// The queue the platform's transport sends from.
	pub fn queue(&mut self) -> &mut Queue {
		&mut self.queue
	}

	/// The platform the client was made with.
	pub fn platform(&self) -> &P {
		&self.platform
	}

	/// Records that `name` happened, with `properties`. Properties given to
	/// [`register`](Self::register) are added, and those given here win.
	pub fn capture(&mut self, name: impl Into<String>, mut properties: Properties) {
		if self.opted_out {
			return;
		}
		for (name, value) in &self.registered {
			if !properties.iter().any(|(have, _)| have == name) {
				properties.push((name.clone(), value.clone()));
			}
		}
		if !self.groups.is_empty() {
			properties.push(("$groups".into(), Value::Map(self.groups.clone())));
		}
		self.record(Body::Event {
			name: name.into(),
			properties,
		});
	}

	/// The person signed in as `distinct_id`, the app's own id for them:
	/// events from now on are theirs, and so are the ones recorded under
	/// the made-up id before. `set` are properties of the person.
	pub fn identify(&mut self, distinct_id: impl Into<String>, set: Properties) {
		let distinct_id = distinct_id.into();
		if self.opted_out || distinct_id.is_empty() {
			return;
		}
		if distinct_id == self.identity.distinct_id {
			if !set.is_empty() {
				self.set(set, Vec::new());
			}
			return;
		}
		let anon_distinct_id = std::mem::replace(&mut self.identity.distinct_id, distinct_id);
		self.identity.identified = true;
		self.record(Body::Identify {
			anon_distinct_id,
			set,
			set_once: Vec::new(),
		});
	}

	/// `alias` is another id for the person.
	pub fn alias(&mut self, alias: impl Into<String>) {
		if self.opted_out {
			return;
		}
		self.record(Body::Alias {
			alias: alias.into(),
		});
	}

	/// Sets the person's properties: `set` always, `set_once` only where
	/// the person has no value yet.
	pub fn set(&mut self, set: Properties, set_once: Properties) {
		if self.opted_out {
			return;
		}
		self.record(Body::Set {
			set,
			set_once,
			unset: Vec::new(),
		});
	}

	/// Removes the person's properties named in `names`.
	pub fn unset(&mut self, names: Vec<String>) {
		if self.opted_out {
			return;
		}
		self.record(Body::Set {
			set: Vec::new(),
			set_once: Vec::new(),
			unset: names,
		});
	}

	/// Events from now on are about the group `key` of type `group_type`,
	/// such as a company, as well as the person; `set` are the group's
	/// properties.
	pub fn group(
		&mut self,
		group_type: impl Into<String>,
		key: impl Into<String>,
		set: Properties,
	) {
		let (group_type, key) = (group_type.into(), key.into());
		if self.opted_out {
			return;
		}
		self.groups.retain(|(have, _)| *have != group_type);
		self.groups
			.push((group_type.clone(), Value::Text(key.clone())));
		self.record(Body::GroupIdentify {
			group_type,
			group_key: key,
			set,
		});
	}

	/// Adds `properties` to every event from now on, replacing any with the
	/// same names.
	pub fn register(&mut self, properties: Properties) {
		for (name, value) in properties {
			self.registered.retain(|(have, _)| *have != name);
			self.registered.push((name, value));
		}
	}

	/// [`register`](Self::register), but only names not already
	/// registered.
	pub fn register_once(&mut self, properties: Properties) {
		for (name, value) in properties {
			if !self.registered.iter().any(|(have, _)| *have == name) {
				self.registered.push((name, value));
			}
		}
	}

	/// Stops adding the property `name` to events.
	pub fn unregister(&mut self, name: &str) {
		self.registered.retain(|(have, _)| have != name);
	}

	/// The person signed out: a new made-up id, a new session, and no
	/// registered properties or groups. What's queued is still sent.
	pub fn reset(&mut self) {
		self.identity = Identity {
			distinct_id: anonymous_id(&self.platform),
			identified: false,
		};
		self.session = None;
		self.registered.clear();
		self.groups.clear();
	}

	/// Stops recording, and drops what's queued.
	pub fn opt_out(&mut self) {
		self.opted_out = true;
		self.queue.clear();
	}

	/// Records again after [`opt_out`](Self::opt_out).
	pub fn opt_in(&mut self) {
		self.opted_out = false;
	}

	/// Whether the person opted out.
	pub fn is_opted_out(&self) -> bool {
		self.opted_out
	}

	/// The current session's id, starting a new session when the last one
	/// ended.
	pub fn session_id(&mut self) -> [u8; 16] {
		let now = self.platform.now();
		let ended = self.session.as_ref().is_none_or(|session| {
			now - session.last > self.config.session_idle
				|| now - session.started > self.config.session_max
		});
		if ended {
			self.session = Some(Session {
				id: wire::uuid_v7(now, random(&self.platform)),
				started: now,
				last: now,
			});
		}
		let session = self.session.as_mut().expect("just started");
		session.last = now;
		session.id
	}

	fn record(&mut self, body: Body) {
		let now = self.platform.now();
		let session_id = Some(self.session_id());
		self.queue.push(
			Entry {
				uuid: wire::uuid_v7(now, random(&self.platform)),
				distinct_id: self.identity.distinct_id.clone(),
				session_id,
				context: 0,
				body,
			},
			now,
		);
	}
}

/// Ten random bytes, for the random part of a UUID v7.
pub fn random(platform: &impl Platform) -> [u8; 10] {
	let mut bytes = [0; 10];
	platform.fill_random(&mut bytes);
	bytes
}

/// A new made-up id: a UUID v7, written the usual way.
pub fn anonymous_id(platform: &impl Platform) -> String {
	format_uuid(&wire::uuid_v7(platform.now(), random(platform)))
}

/// `uuid` written as `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
pub fn format_uuid(uuid: &[u8; 16]) -> String {
	const HEX: &[u8; 16] = b"0123456789abcdef";
	let mut out = String::with_capacity(36);
	for (index, byte) in uuid.iter().enumerate() {
		if matches!(index, 4 | 6 | 8 | 10) {
			out.push('-');
		}
		out.push(HEX[usize::from(byte >> 4)] as char);
		out.push(HEX[usize::from(byte & 0x0F)] as char);
	}
	out
}

/// Properties, written `props! { "plan" => "pro", "seats" => 3 }`.
#[macro_export]
macro_rules! props {
	() => { ::std::vec::Vec::<(::std::string::String, $crate::wire::Value)>::new() };
	($($name:expr => $value:expr),+ $(,)?) => {
		::std::vec![$((::std::string::String::from($name), $crate::wire::Value::from($value))),+]
	};
}

#[cfg(test)]
mod tests {
	use std::cell::Cell;

	use super::*;

	struct Fake {
		now: Cell<i64>,
		counter: Cell<u8>,
	}

	impl Platform for &Fake {
		fn now(&self) -> i64 {
			self.now.get()
		}

		fn fill_random(&self, bytes: &mut [u8]) {
			self.counter.set(self.counter.get().wrapping_add(1));
			bytes.fill(self.counter.get());
		}
	}

	fn fake() -> Fake {
		Fake {
			now: Cell::new(1_762_646_400_000),
			counter: Cell::new(0),
		}
	}

	fn sent(client: &mut Client<&Fake>) -> wire::Batch {
		let now = client.platform().now();
		let pending = client.queue().take(now, [0; 10]).unwrap();
		client.queue().settle(Outcome::Stored, now, 0.0);
		wire::decode(&pending.body, &wire::Limits::default()).unwrap()
	}

	#[test]
	fn identify_links_the_made_up_id_and_reset_makes_a_new_one() {
		let platform = fake();
		let mut client = Client::new(&platform, Config::default(), props! { "$lib" => "test" });
		let anonymous = client.identity().distinct_id.clone();
		assert_eq!(anonymous.len(), 36);
		assert!(!client.identity().identified);

		client.capture("opened", props! {});
		client.identify("user-1", props! { "email" => "ada@example.com" });
		client.capture("bought", props! { "plan" => "pro" });
		let batch = sent(&mut client);
		assert_eq!(batch.entries[0].distinct_id, anonymous);
		assert_eq!(batch.entries[1].distinct_id, "user-1");
		assert_eq!(
			batch.entries[1].body,
			Body::Identify {
				anon_distinct_id: anonymous.clone(),
				set: props! { "email" => "ada@example.com" },
				set_once: Vec::new(),
			}
		);
		assert_eq!(batch.entries[2].distinct_id, "user-1");

		client.reset();
		assert_ne!(client.identity().distinct_id, anonymous);
		assert!(!client.identity().identified);
	}

	#[test]
	fn registered_properties_and_groups_go_on_every_event() {
		let platform = fake();
		let mut client = Client::new(&platform, Config::default(), props! {});
		client.register(props! { "flavour" => "beta", "plan" => "free" });
		client.register_once(props! { "flavour" => "ignored" });
		client.group("company", "acme", props! { "name" => "Acme" });
		client.capture("clicked", props! { "plan" => "pro" });
		client.unregister("flavour");
		client.capture("clicked again", props! {});

		let batch = sent(&mut client);
		let Body::Event { properties, .. } = &batch.entries[1].body else {
			panic!("not an event");
		};
		assert_eq!(
			properties,
			&props! {
				"plan" => "pro",
				"flavour" => "beta",
				"$groups" => Value::Map(props! { "company" => "acme" }),
			}
		);
		let Body::Event { properties, .. } = &batch.entries[2].body else {
			panic!("not an event");
		};
		assert_eq!(
			properties,
			&props! { "plan" => "free", "$groups" => Value::Map(props! { "company" => "acme" }) }
		);
	}

	#[test]
	fn sessions_end_when_idle_or_too_long() {
		let platform = fake();
		let mut client = Client::new(&platform, Config::default(), props! {});
		let first = client.session_id();
		platform.now.set(platform.now.get() + 29 * 60 * 1_000);
		assert_eq!(client.session_id(), first, "still active");
		platform.now.set(platform.now.get() + 31 * 60 * 1_000);
		let second = client.session_id();
		assert_ne!(second, first, "idle too long");
		for _ in 0..50 {
			platform.now.set(platform.now.get() + 29 * 60 * 1_000);
			if client.session_id() != second {
				return;
			}
		}
		panic!("a session outlived its maximum");
	}

	#[test]
	fn opting_out_records_nothing_and_drops_the_queue() {
		let platform = fake();
		let mut client = Client::new(&platform, Config::default(), props! {});
		client.capture("before", props! {});
		client.opt_out();
		client.capture("during", props! {});
		assert!(client.queue().is_empty());
		client.opt_in();
		client.capture("after", props! {});
		assert_eq!(client.queue().len(), 1);
	}

	#[test]
	fn uuids_are_written_the_usual_way() {
		assert_eq!(
			format_uuid(&[
				0x01, 0x9a, 0x5f, 0x3c, 0, 0, 0x70, 0, 0x80, 0, 0, 0, 0, 0, 0, 0xff
			]),
			"019a5f3c-0000-7000-8000-0000000000ff"
		);
	}
}
