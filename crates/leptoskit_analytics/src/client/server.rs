//! The client for an app's server: events about many people, each call
//! naming whose, sent in the background to the analytics service's private
//! URL, signed as the app.
//!
//! Recording never waits: [`Analytics::capture`] queues and returns, and a
//! task sends batches every second, or as soon as a batch's worth is
//! waiting. [`Analytics::flush`] waits until what's queued is stored, for
//! a process about to exit.

use std::hash::BuildHasher as _;
use std::hash::Hasher as _;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;
use std::time::SystemTime;

use super::Limits;
use super::Outcome;
use super::Platform;
use super::Queue;
use crate::SERVICE;
use crate::wire;
use crate::wire::Body;
use crate::wire::Entry;
use crate::wire::Properties;
use crate::wire::Value;

/// How the server's client batches.
#[derive(Clone, Debug)]
pub struct Config {
	pub limits: Limits,
	/// How often waiting events are sent.
	pub interval: Duration,
	/// Events are sent as soon as this many are waiting.
	pub flush_at: usize,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			limits: Limits::default(),
			interval: Duration::from_secs(1),
			flush_at: 100,
		}
	}
}

/// The server's clock and random numbers, without a crate for either.
#[derive(Clone, Copy, Debug, Default)]
pub struct System;

impl Platform for System {
	fn now(&self) -> i64 {
		SystemTime::now()
			.duration_since(SystemTime::UNIX_EPOCH)
			.map_or(0, |since| since.as_millis() as i64)
	}

	fn fill_random(&self, bytes: &mut [u8]) {
		// Each `RandomState` is keyed differently, from the process's random
		// seed, so its hashes are unpredictable and don't repeat.
		for chunk in bytes.chunks_mut(8) {
			let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
			hasher.write_u8(0);
			chunk.copy_from_slice(&hasher.finish().to_le_bytes()[..chunk.len()]);
		}
	}
}

/// Why [`Analytics::flush`] didn't store everything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlushError {
	/// Events still waiting, to be retried later.
	pub waiting: usize,
}

impl std::fmt::Display for FlushError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"the analytics service didn't store {} events; they'll be retried",
			self.waiting
		)
	}
}

impl std::error::Error for FlushError {}

struct Shared {
	queue: Mutex<Queue>,
	/// Held while a batch is sent, so [`Analytics::flush`] waits for the
	/// background task's batch rather than skipping it.
	sending: tokio::sync::Mutex<()>,
	wake: tokio::sync::Notify,
	url: String,
	/// Where the event catalogue's `register_events` operation is.
	register_url: String,
	token: Option<String>,
	http: reqwest::Client,
	flush_at: usize,
	/// Whether this program's typed events are in the service's catalogue.
	registered: std::sync::atomic::AtomicBool,
}

/// Records events from the app's server. Clones share one queue.
#[derive(Clone)]
pub struct Analytics {
	shared: Arc<Shared>,
}

impl Analytics {
	/// The client `leptoskit dev` and `leptoskit serve` configure: the
	/// service's private URL from `LEPTOSKIT_SERVICE_ANALYTICS_URL`, signed
	/// with `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`. `None` when the URL isn't
	/// set: the app doesn't run the service.
	///
	/// Starts a task, so it must be called within a Tokio runtime.
	pub fn from_env() -> Option<Self> {
		let read = |suffix: &str| {
			std::env::var(format!(
				"LEPTOSKIT_SERVICE_{}_{suffix}",
				SERVICE.to_uppercase()
			))
			.ok()
			.filter(|value| !value.trim().is_empty())
		};
		Some(Self::new(&read("URL")?, read("TOKEN"), Config::default()))
	}

	/// A client sending to the service at `url`, its private URL, with
	/// `token` when it's set. Starts a task, so it must be called within a
	/// Tokio runtime.
	pub fn new(url: &str, token: Option<String>, config: Config) -> Self {
		let context = vec![
			("$lib".into(), Value::Text("leptoskit-server".into())),
			(
				"$lib_version".into(),
				Value::Text(env!("CARGO_PKG_VERSION").into()),
			),
		];
		let shared = Arc::new(Shared {
			queue: Mutex::new(Queue::new(config.limits, context)),
			sending: tokio::sync::Mutex::new(()),
			wake: tokio::sync::Notify::new(),
			url: format!(
				"{}/_leptoskit/services/{SERVICE}/v1/batch",
				url.trim_end_matches('/')
			),
			register_url: format!(
				"{}/_leptoskit/services/{SERVICE}/api/register_events",
				url.trim_end_matches('/')
			),
			registered: std::sync::atomic::AtomicBool::new(false),
			token,
			http: reqwest::Client::new(),
			flush_at: config.flush_at.max(1),
		});
		tokio::spawn(run(Arc::downgrade(&shared), config.interval));
		Self { shared }
	}

	/// Records that `name` happened to `distinct_id`.
	pub fn capture(&self, distinct_id: &str, name: impl Into<String>, properties: Properties) {
		self.record(
			distinct_id,
			Body::Event {
				name: name.into(),
				properties,
			},
		);
	}

	/// Records a typed event for `distinct_id`: [`capture`](Self::capture)
	/// with its name and properties.
	pub fn capture_event<E: crate::Event>(&self, distinct_id: &str, event: E) {
		self.queue().remember_schema(E::SCHEMA_ID, E::FIELDS);
		self.record(
			distinct_id,
			Body::TypedEvent {
				name: E::NAME.into(),
				schema_id: E::SCHEMA_ID,
				values: event.into_values(),
				properties: Vec::new(),
			},
		);
	}

	/// `distinct_id` is the person `anon_distinct_id` was, now signed in;
	/// `set` are their properties.
	pub fn identify(&self, distinct_id: &str, anon_distinct_id: &str, set: Properties) {
		self.record(
			distinct_id,
			Body::Identify {
				anon_distinct_id: anon_distinct_id.into(),
				set,
				set_once: Vec::new(),
			},
		);
	}

	/// `alias` is another id for `distinct_id`.
	pub fn alias(&self, distinct_id: &str, alias: &str) {
		self.record(
			distinct_id,
			Body::Alias {
				alias: alias.into(),
			},
		);
	}

	/// Sets `distinct_id`'s properties: `set` always, `set_once` only where
	/// they have no value yet.
	pub fn set(&self, distinct_id: &str, set: Properties, set_once: Properties) {
		self.record(
			distinct_id,
			Body::Set {
				set,
				set_once,
				unset: Vec::new(),
			},
		);
	}

	/// Sets the properties of the group `key` of type `group_type`.
	pub fn group_identify(&self, group_type: &str, key: &str, set: Properties) {
		self.record(
			&format!("${group_type}_{key}"),
			Body::GroupIdentify {
				group_type: group_type.into(),
				group_key: key.into(),
				set,
			},
		);
	}

	/// How many events are waiting to be sent.
	pub fn waiting(&self) -> usize {
		self.queue().len()
	}

	/// Sends what's waiting, and waits until it's stored.
	///
	/// # Errors
	///
	/// When the service can't be reached or asks to retry later; the events
	/// stay queued and the background task retries them.
	pub async fn flush(&self) -> Result<(), FlushError> {
		send_waiting(&self.shared).await;
		let queue = self.queue();
		if queue.is_empty() && !queue.is_sending() {
			Ok(())
		} else {
			Err(FlushError {
				waiting: queue.len(),
			})
		}
	}

	fn record(&self, distinct_id: &str, body: Body) {
		let now = System.now();
		let entry = Entry {
			uuid: wire::uuid_v7(now, super::random(&System)),
			distinct_id: distinct_id.into(),
			session_id: None,
			context: 0,
			body,
		};
		let waiting = {
			let mut queue = self.queue();
			queue.push(entry, now);
			queue.len()
		};
		if waiting >= self.shared.flush_at {
			self.shared.wake.notify_one();
		}
	}

	fn queue(&self) -> std::sync::MutexGuard<'_, Queue> {
		self.shared
			.queue
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
	}
}

/// Sends batches every `interval`, or when woken, until every client is
/// dropped.
async fn run(shared: std::sync::Weak<Shared>, interval: Duration) {
	loop {
		let Some(strong) = shared.upgrade() else {
			return;
		};
		// Woken early, or on the interval; holding only a weak reference
		// while asleep, so dropping the last client ends the task.
		let wake = async {
			tokio::select! {
				() = strong.wake.notified() => {}
				() = tokio::time::sleep(interval) => {}
			}
		};
		wake.await;
		send_waiting(&strong).await;
		drop(strong);
	}
}

/// The most schemas one registration sends, as the service accepts.
const SCHEMAS_PER_REGISTRATION: usize = 1_000;

/// Registers the typed events this server was built with in the service's
/// event catalogue, as the app, until the service accepts them. Until then
/// typed events whose schemas it doesn't know are sent again with names.
async fn register_events(shared: &Shared) {
	use std::sync::atomic::Ordering;

	if shared.registered.load(Ordering::Relaxed) {
		return;
	}
	#[cfg(feature = "derive")]
	{
		let schemas: Vec<serde_json::Value> = crate::compiled_events()
			.map(|schema| {
				serde_json::json!({
					"schema_id": schema.schema_id,
					"name": schema.name,
					"fields": schema.fields,
				})
			})
			.collect();
		for chunk in schemas.chunks(SCHEMAS_PER_REGISTRATION) {
			let mut request = shared
				.http
				.post(&shared.register_url)
				.header("content-type", "application/json")
				.body(serde_json::json!({ "schemas": chunk }).to_string());
			if let Some(token) = &shared.token {
				request = request.bearer_auth(token);
			}
			match request.send().await {
				Ok(response) if response.status().is_success() => {}
				_ => return,
			}
		}
	}
	#[cfg(not(feature = "derive"))]
	let _ = SCHEMAS_PER_REGISTRATION;
	shared.registered.store(true, Ordering::Relaxed);
}

/// The entries a stored batch's answer says were refused, by index and
/// reason.
fn refusals(answer: &[u8]) -> Vec<(u32, String)> {
	let Ok(answer) = serde_json::from_slice::<serde_json::Value>(answer) else {
		return Vec::new();
	};
	answer["refused"]
		.as_array()
		.map(|refused| {
			refused
				.iter()
				.filter_map(|refusal| {
					let index = u32::try_from(refusal["index"].as_u64()?).ok()?;
					Some((index, refusal["reason"].as_str()?.to_owned()))
				})
				.collect()
		})
		.unwrap_or_default()
}

/// Sends batches until nothing waits or the service asks to wait.
async fn send_waiting(shared: &Shared) {
	let _sending = shared.sending.lock().await;
	register_events(shared).await;
	loop {
		let pending = {
			let mut queue = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
			queue.take(System.now(), super::random(&System))
		};
		let Some(pending) = pending else {
			return;
		};

		let mut request = shared
			.http
			.post(&shared.url)
			.header("content-type", wire::CONTENT_TYPE)
			.body(pending.body);
		if let Some(token) = &shared.token {
			request = request.bearer_auth(token);
		}
		let (outcome, refused) = match request.send().await {
			Ok(response) => {
				let outcome = Outcome::from_status(
					response.status().as_u16(),
					response
						.headers()
						.get("retry-after")
						.and_then(|value| value.to_str().ok()),
				);
				let refused = if outcome == Outcome::Stored {
					refusals(&response.bytes().await.unwrap_or_default())
				} else {
					Vec::new()
				};
				(outcome, refused)
			}
			Err(_) => (Outcome::Retry { after: None }, Vec::new()),
		};

		let mut jitter = [0_u8; 1];
		System.fill_random(&mut jitter);
		let jitter = f64::from(jitter[0]) / 255.0;
		let mut queue = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
		if outcome == Outcome::Stored {
			let refused: Vec<(u32, &str)> = refused
				.iter()
				.map(|(index, reason)| (*index, reason.as_str()))
				.collect();
			queue.settle_with_refusals(&refused, System.now(), jitter);
		} else {
			queue.settle(outcome, System.now(), jitter);
		}
		if matches!(outcome, Outcome::Retry { .. }) {
			return;
		}
	}
}
