//! The client in a native app's Wasm guest: one per app, sending to the
//! app's server through the HTTP plugin, and keeping what it hasn't sent in
//! the app's private files, so events recorded offline, or just before the
//! app was closed, are sent the next time it runs.
//!
//! [`init`] starts it, once the app knows its server's origin; the
//! functions below record through it, and do nothing before it. Every
//! second it sends if [`Options::flush_at`] events are waiting or
//! [`Options::interval`] has passed, and saves the queue if it changed.
//! Call [`flush`] when the app goes to the background.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use leptoskit_plugin_filesystem::MAX_FILE_BYTES;
use leptoskit_plugin_filesystem::guest::Filesystem;
use leptoskit_plugin_http::Header;
use leptoskit_plugin_http::Method;
use leptoskit_plugin_http::guest::HttpClient;

use super::Client;
use super::Config;
use super::Identity;
use super::Outcome;
use super::Platform;
use super::persistence;
use crate::SERVICE;
use crate::wire;
use crate::wire::Properties;
use crate::wire::Value;

/// How the native client sends and what it keeps.
#[derive(Clone, Debug)]
pub struct Options {
	/// The app's server, such as `https://app.example`.
	pub origin: String,
	pub config: Config,
	/// Waiting events are sent at least this often.
	pub interval: Duration,
	/// Events are sent as soon as this many are waiting.
	pub flush_at: usize,
	/// Whether the person, their consent, and unsent events are kept in the
	/// app's files.
	pub persist: bool,
	/// Added to every batch: the app's version, the device's model, the
	/// operating system.
	pub context: Properties,
}

impl Options {
	/// Sends to the app's server at `origin`, with `PostHog`'s mobile
	/// defaults: every 30 seconds or 20 events.
	pub fn new(origin: impl Into<String>) -> Self {
		Self {
			origin: origin.into(),
			config: Config::default(),
			interval: Duration::from_secs(30),
			flush_at: 20,
			persist: true,
			context: Vec::new(),
		}
	}
}

/// The host's clock, and random numbers from a generator seeded by the
/// crypto plugin when the app has it.
#[derive(Debug, Default)]
pub struct Guest {
	state: Cell<u64>,
}

impl Platform for Guest {
	fn now(&self) -> i64 {
		leptoskit_query::now_ms() as i64
	}

	fn fill_random(&self, bytes: &mut [u8]) {
		for chunk in bytes.chunks_mut(8) {
			// SplitMix64: fast, and never repeats within 2^64 calls.
			let state = self.state.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
			self.state.set(state);
			let mut z = state;
			z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
			z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
			z ^= z >> 31;
			chunk.copy_from_slice(&z.to_le_bytes()[..chunk.len()]);
		}
	}
}

struct State {
	client: Client<Guest>,
	options: Options,
	endpoint: String,
	/// The queue version and checkpoint committed by the last successful save.
	saved: (u64, Option<persistence::Checkpoint>),
	saving: Rc<persistence::Gate>,
}

thread_local! {
	static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

const DIRECTORY: &str = "leptoskit_analytics";
const IDENTITY: &str = "leptoskit_analytics/identity";
const OPTED_OUT: &str = "leptoskit_analytics/opted_out";

/// Starts the app's client: recognises the person from last time, restores
/// what wasn't sent, and starts sending. Calling it again does nothing.
/// If existing persistent queue files cannot be read, initialization leaves
/// them intact and may be tried again; [`distinct_id`] remains `None`.
pub async fn init(options: Options) {
	if STATE.with_borrow(Option::is_some) {
		return;
	}
	let guest = Guest::default();
	guest.state.set(seed().await);

	let files = options.persist.then(|| Filesystem::new().ok()).flatten();
	let read = async |path: &str| {
		match files {
			Some(files) => files.read(path).await.ok().map(|bytes| bytes.to_vec()),
			None => None,
		}
	};

	let identity = read(IDENTITY).await.and_then(|bytes| {
		let stored = String::from_utf8(bytes).ok()?;
		let (kind, id) = stored.split_once(':')?;
		(!id.is_empty()).then(|| {
			Identity {
				distinct_id: id.into(),
				identified: kind == "i",
			}
		})
	});
	let opted_out = read(OPTED_OUT).await.is_some();

	let mut context: Properties = vec![
		("$lib".into(), Value::Text("leptoskit-native".into())),
		(
			"$lib_version".into(),
			Value::Text(env!("CARGO_PKG_VERSION").into()),
		),
	];
	context.extend(options.context.iter().cloned());
	let mut client = match identity {
		Some(identity) => Client::restore(guest, options.config.clone(), context, identity),
		None => Client::new(guest, options.config.clone(), context),
	};

	let mut checkpoint = None;
	if opted_out {
		client.opt_out();
		// Commit an empty checkpoint before opt-in can remove the marker.
		// Old queue files must never reappear after an opted-out restart.
		if let Some(files) = files {
			let Ok(empty) = persistence::save(&Disk(files), None, &[]).await else {
				return;
			};
			checkpoint = Some(empty);
		}
	} else if let Some(files) = files {
		let Ok((stored, segments)) = persistence::load(&Disk(files)).await else {
			return;
		};
		checkpoint = stored;
		for segment in segments {
			if client.queue().restore(&segment).is_err() {
				return;
			}
		}
	}

	let endpoint = format!(
		"{}/_leptoskit/services/{SERVICE}/v1/batch",
		options.origin.trim_end_matches('/')
	);
	let version = client.queue().version();
	if STATE.with_borrow(Option::is_some) {
		return;
	}
	STATE.with_borrow_mut(|state| {
		*state = Some(State {
			client,
			options,
			endpoint,
			saved: (version, checkpoint),
			saving: Rc::default(),
		});
	});
	save_identity().await;
	any_spawner::Executor::spawn_local(run());
}

/// Records that `name` happened.
pub fn capture(name: &str, properties: Properties) {
	with(|client| client.capture(name, properties));
}

/// Records a typed event with its declared name and fields.
pub fn capture_event<E: crate::Event>(event: E) {
	with(|client| client.capture_event(event));
}

/// Records that the person saw the screen `name`.
pub fn capture_screen(name: &str, mut properties: Properties) {
	properties.push(("$screen_name".into(), Value::Text(name.into())));
	capture("$screen", properties);
}

/// The person signed in as `distinct_id`; see [`Client::identify`].
pub fn identify(distinct_id: &str, set: Properties) {
	with(|client| client.identify(distinct_id, set));
	any_spawner::Executor::spawn_local(save_identity());
}

/// `alias` is another id for the person.
pub fn alias(alias: &str) {
	with(|client| client.alias(alias));
}

/// Sets the person's properties; see [`Client::set`].
pub fn set(set: Properties, set_once: Properties) {
	with(|client| client.set(set, set_once));
}

/// Events from now on are about this group too; see [`Client::group`].
pub fn group(group_type: &str, key: &str, set: Properties) {
	with(|client| client.group(group_type, key, set));
}

/// Adds `properties` to every event from now on.
pub fn register(properties: Properties) {
	with(|client| client.register(properties));
}

/// [`register`], but only names not already registered.
pub fn register_once(properties: Properties) {
	with(|client| client.register_once(properties));
}

/// Stops adding `name` to events.
pub fn unregister(name: &str) {
	with(|client| client.unregister(name));
}

/// The person signed out: someone new from now on.
pub fn reset() {
	with(Client::reset);
	any_spawner::Executor::spawn_local(save_identity());
}

/// Stops recording, drops what's waiting, and remembers the choice.
pub fn opt_out() {
	with(Client::opt_out);
	any_spawner::Executor::spawn_local(async {
		if let Some(files) = files() {
			let _ = files.mkdir(DIRECTORY).await;
			let _ = files.write(OPTED_OUT, b"1").await;
		}
		save().await;
	});
}

/// Records again after [`opt_out`].
pub fn opt_in() {
	with(Client::opt_in);
	any_spawner::Executor::spawn_local(async {
		if let Some(files) = files() {
			let _ = files.remove(OPTED_OUT).await;
		}
	});
}

/// Who events are recorded as, once [`init`] has run.
pub fn distinct_id() -> Option<String> {
	STATE.with_borrow(|state| {
		state
			.as_ref()
			.map(|state| state.client.identity().distinct_id.clone())
	})
}

/// Sends what's waiting now, and saves what couldn't be sent: call it when
/// the app goes to the background.
pub async fn flush() {
	send_waiting().await;
	save().await;
}

fn with(f: impl FnOnce(&mut Client<Guest>)) {
	STATE.with_borrow_mut(|state| {
		if let Some(state) = state {
			f(&mut state.client);
		}
	});
}

/// The app's files, when the client may keep anything in them.
fn files() -> Option<Filesystem> {
	let persist =
		STATE.with_borrow(|state| state.as_ref().is_some_and(|state| state.options.persist));
	persist.then(|| Filesystem::new().ok()).flatten()
}

/// A seed for ids: from the crypto plugin, or from the clock.
async fn seed() -> u64 {
	let random = match leptoskit_plugin_crypto::guest::Crypto::new() {
		Ok(crypto) => crypto.random_bytes(8).await.ok(),
		Err(_) => None,
	};
	match random {
		Some(bytes) if bytes.len() == 8 => {
			u64::from_le_bytes(bytes[..8].try_into().expect("8 bytes"))
		}
		_ => leptoskit_query::now_ms().rotate_left(17) ^ 0xA076_1D64_78BD_642F,
	}
}

async fn save_identity() {
	let saved = STATE.with_borrow(|state| {
		state.as_ref().map(|state| {
			let identity = state.client.identity();
			let kind = if identity.identified { "i" } else { "a" };
			format!("{kind}:{}", identity.distinct_id)
		})
	});
	if let (Some(saved), Some(files)) = (saved, files()) {
		let _ = files.mkdir(DIRECTORY).await;
		let _ = files.write(IDENTITY, saved.as_bytes()).await;
	}
}

struct Disk(Filesystem);

impl persistence::Store for Disk {
	async fn mkdir(&self, path: &str) -> Result<(), ()> {
		self.0.mkdir(path).await.map_err(|_| ())
	}

	async fn list(&self, path: &str) -> Result<Vec<String>, ()> {
		self.0.list(path).await.map_err(|_| ())
	}

	async fn read(&self, path: &str) -> Result<Vec<u8>, ()> {
		self.0
			.read(path)
			.await
			.map(|bytes| bytes.to_vec())
			.map_err(|_| ())
	}

	async fn write(&self, path: &str, bytes: &[u8]) -> Result<(), ()> {
		self.0.write(path, bytes).await.map_err(|_| ())
	}

	async fn remove(&self, path: &str) -> Result<(), ()> {
		self.0.remove(path).await.map_err(|_| ())
	}
}

/// Saves only to the inactive bank and advances the saved version after an
/// atomic manifest commit. A failed write is retried on the next save.
async fn save() {
	let Some(files) = files() else {
		return;
	};
	let Some(saving) = STATE.with_borrow(|state| state.as_ref().map(|state| state.saving.clone()))
	else {
		return;
	};
	let _guard = saving.lock().await;
	let snapshot = STATE.with_borrow_mut(|state| {
		let state = state.as_mut()?;
		let version = state.client.queue().version();
		(version != state.saved.0).then(|| {
			(
				version,
				state.client.queue().snapshot(MAX_FILE_BYTES),
				state.saved.1,
			)
		})
	});
	let Some((version, segments, previous)) = snapshot else {
		return;
	};
	let Ok(checkpoint) = persistence::save(&Disk(files), previous, &segments).await else {
		return;
	};
	STATE.with_borrow_mut(|state| {
		if let Some(state) = state {
			state.saved = (version, Some(checkpoint));
		}
	});
}

/// Every second: send when enough waits or the interval has passed, then
/// save. Ends if the host has no timer.
async fn run() {
	let mut sent_at = leptoskit_query::now_ms();
	loop {
		if leptoskit_bridge::guest::sleep(Duration::from_secs(1))
			.await
			.is_err()
		{
			return;
		}
		let due = STATE.with_borrow_mut(|state| {
			state.as_mut().is_some_and(|state| {
				let waiting = state.client.queue().len();
				let elapsed = leptoskit_query::now_ms().saturating_sub(sent_at);
				waiting > 0
					&& (waiting >= state.options.flush_at
						|| u128::from(elapsed) >= state.options.interval.as_millis())
			})
		});
		if due {
			send_waiting().await;
			sent_at = leptoskit_query::now_ms();
		}
		save().await;
	}
}

fn take() -> Option<(super::Pending, String)> {
	STATE.with_borrow_mut(|state| {
		let state = state.as_mut()?;
		let now = state.client.platform().now();
		let random = super::random(state.client.platform());
		let pending = state.client.queue().take(now, random)?;
		Some((pending, state.endpoint.clone()))
	})
}

async fn send_waiting() {
	while let Some((pending, endpoint)) = take() {
		let (outcome, refused) = post(endpoint, pending.body).await;
		STATE.with_borrow_mut(|state| {
			if let Some(state) = state {
				let mut jitter = [0_u8; 1];
				state.client.platform().fill_random(&mut jitter);
				let now = state.client.platform().now();
				let jitter = f64::from(jitter[0]) / 255.0;
				if outcome == Outcome::Stored {
					let refused: Vec<(u32, &str)> = refused
						.iter()
						.map(|(index, reason)| (*index, reason.as_str()))
						.collect();
					state
						.client
						.queue()
						.settle_with_refusals(&refused, now, jitter);
				} else {
					state.client.queue().settle(outcome, now, jitter);
				}
			}
		});
		if matches!(outcome, Outcome::Retry { .. }) {
			return;
		}
	}
}

async fn post(url: String, body: Vec<u8>) -> (Outcome, Vec<(u32, String)>) {
	let Ok(http) = HttpClient::connect() else {
		return (Outcome::Retry { after: None }, Vec::new());
	};
	let request = leptoskit_plugin_http::Request {
		method: Method::Post,
		url,
		headers: vec![Header {
			name: "content-type".into(),
			value: wire::CONTENT_TYPE.as_bytes().to_vec(),
		}],
		body,
	};
	match http.execute(request).await {
		Ok(response) => {
			let retry_after = response
				.headers
				.iter()
				.find(|header| header.name.eq_ignore_ascii_case("retry-after"))
				.and_then(|header| std::str::from_utf8(&header.value).ok());
			super::acknowledge(
				Outcome::from_status(response.status, retry_after),
				&response.body,
			)
		}
		Err(_) => (Outcome::Retry { after: None }, Vec::new()),
	}
}
