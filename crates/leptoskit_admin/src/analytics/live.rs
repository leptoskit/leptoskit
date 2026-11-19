//! The live tail, shared: one subscription to the service's `live_events`
//! stream for the whole admin, fanned out to every open page.
//!
//! The admin's server holds the subscription, because only it has the
//! service's token. It opens the service's live connection over
//! server-sent events (`GET live/events`, then `POST live/frames` with
//! `hello` and `stream`), signed as the app, keeps the stream's credit
//! topped up, and reopens from the last cursor when the connection drops.
//! Pages read it from the admin as server-sent events of their own.
//!
//! Beside the events, it keeps a count per second for the last minute: the
//! top bar's pulse and the Live page's rate.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::StreamExt as _;
use futures::stream::BoxStream;
use leptoskit_admin_pages::model::live::Arrival;
use leptoskit_admin_pages::ops::Live;
use leptoskit_analytics::service::query::TailItem;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use serde::Serialize;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

/// The stream's id on the admin's connection: it opens just the one.
const STREAM: u32 = 1;

/// How many items the service may send ahead of the admin.
const CREDITS: u32 = 512;

/// How many seconds the pulse remembers.
pub(crate) const WINDOW: usize = 60;

/// Whether the tail is flowing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub(crate) enum Status {
	/// Not asked for yet, or connecting.
	Connecting,
	/// Subscribed.
	Live,
	/// The connection dropped or was refused; the hub is trying again.
	Disconnected(String),
}

/// The pulse: how fast events arrive.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Pulse {
	pub status: Status,
	/// Events per second over the last ten seconds.
	pub per_second: f64,
	/// Events in the last minute.
	pub per_minute: u64,
	/// Events in each of the last sixty seconds, oldest first.
	pub seconds: Vec<u64>,
	/// The last event's sequence number: a page that saw a smaller one has
	/// new data to show.
	pub latest: u64,
}

impl Pulse {
	/// The pulse as a page shows it.
	pub(crate) fn shown(self) -> leptoskit_admin_pages::model::Pulse {
		let (live, reason) = match self.status {
			Status::Live => (true, None),
			Status::Connecting => (false, None),
			Status::Disconnected(reason) => (false, Some(reason)),
		};
		leptoskit_admin_pages::model::Pulse {
			per_second: self.per_second,
			per_minute: self.per_minute,
			seconds: self.seconds,
			live,
			reason,
			latest: self.latest,
		}
	}
}

#[derive(Default)]
struct Counts {
	/// `(unix second, events)`, oldest first.
	seconds: VecDeque<(u64, u64)>,
	latest: u64,
}

struct Inner {
	url: String,
	token: String,
	http: reqwest::Client,
	started: AtomicBool,
	sender: broadcast::Sender<Arc<TailItem>>,
	counts: Mutex<Counts>,
	status: Mutex<Status>,
}

/// The shared tail. Clones share one subscription.
#[derive(Clone)]
pub(crate) struct Hub(Arc<Inner>);

fn unix_now() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |elapsed| elapsed.as_secs())
}

impl Hub {
	pub(crate) fn new(url: String, token: String) -> Self {
		let (sender, _) = broadcast::channel(1024);
		Self(Arc::new(Inner {
			url,
			token,
			http: reqwest::Client::new(),
			started: AtomicBool::new(false),
			sender,
			counts: Mutex::new(Counts::default()),
			status: Mutex::new(Status::Connecting),
		}))
	}

	/// Starts the subscription the first time anyone asks for it.
	pub(crate) fn ensure_started(&self) {
		if self.0.started.swap(true, Ordering::SeqCst) {
			return;
		}

		let weak = Arc::downgrade(&self.0);
		tokio::spawn(async move {
			let mut cursor: Option<u64> = None;
			let mut wait = Duration::from_millis(500);

			loop {
				let Some(inner) = weak.upgrade() else {
					return;
				};
				let hub = Self(inner);
				let result = hub.follow(&mut cursor).await;
				let reason = match result {
					Ok(()) => "the service closed the live connection".to_owned(),
					Err(reason) => reason,
				};
				*hub.0.status.lock().unwrap_or_else(PoisonError::into_inner) =
					Status::Disconnected(reason);
				drop(hub);
				tokio::time::sleep(wait).await;
				wait = (wait * 2).min(Duration::from_secs(10));
			}
		});
	}

	/// Every event from now on.
	pub(crate) fn subscribe(&self) -> broadcast::Receiver<Arc<TailItem>> {
		self.ensure_started();
		self.0.sender.subscribe()
	}

	/// How fast events arrive, now.
	pub(crate) fn pulse(&self) -> Pulse {
		self.ensure_started();
		let now = unix_now();
		let counts = self.0.counts.lock().unwrap_or_else(PoisonError::into_inner);
		let mut seconds = vec![0_u64; WINDOW];

		for (second, events) in &counts.seconds {
			let age = now.saturating_sub(*second);

			if let Ok(age) = usize::try_from(age)
				&& age < WINDOW
			{
				seconds[WINDOW - 1 - age] += events;
			}
		}

		let last_ten: u64 = seconds[WINDOW - 10..].iter().sum();
		#[expect(clippy::cast_precision_loss, reason = "a rate for display")]
		let per_second = last_ten as f64 / 10.0;
		Pulse {
			status: self
				.0
				.status
				.lock()
				.unwrap_or_else(PoisonError::into_inner)
				.clone(),
			per_second,
			per_minute: seconds.iter().sum(),
			seconds,
			latest: counts.latest,
		}
	}

	fn record(&self, item: &TailItem) {
		let now = unix_now();
		let mut counts = self.0.counts.lock().unwrap_or_else(PoisonError::into_inner);
		counts.latest = counts.latest.max(item.event.seq);

		match counts.seconds.back_mut() {
			Some((second, events)) if *second == now => *events += 1,
			_ => counts.seconds.push_back((now, 1)),
		}

		while counts
			.seconds
			.front()
			.is_some_and(|(second, _)| now.saturating_sub(*second) >= WINDOW as u64)
		{
			counts.seconds.pop_front();
		}
	}

	fn live_url(&self, path: &str) -> String {
		format!(
			"{}{}{}{path}",
			self.0.url,
			leptoskit_rpc::service_prefix(leptoskit_analytics::SERVICE),
			leptoskit_rpc::live::LIVE_PATH
		)
	}

	async fn post_frames(&self, connection: &str, frames: &[ClientFrame]) -> Result<(), String> {
		let response = self
			.0
			.http
			.post(self.live_url("/frames"))
			.query(&[("connection", connection)])
			.bearer_auth(&self.0.token)
			.json(frames)
			.send()
			.await
			.map_err(|error| format!("couldn't reach the analytics service: {error}"))?;

		if response.status().is_success() {
			Ok(())
		} else {
			let status = response.status();
			let body = response.text().await.unwrap_or_default();
			Err(format!(
				"the analytics service refused the live frames ({status}): {body}"
			))
		}
	}

	/// One connection: open it, subscribe, and pass on every item until it
	/// drops.
	async fn follow(&self, cursor: &mut Option<u64>) -> Result<(), String> {
		let mut response = self
			.0
			.http
			.get(self.live_url("/events"))
			.bearer_auth(&self.0.token)
			.header(reqwest::header::ACCEPT, "text/event-stream")
			.send()
			.await
			.map_err(|error| format!("couldn't reach the analytics service: {error}"))?;

		if !response.status().is_success() {
			let status = response.status();
			let body = response.text().await.unwrap_or_default();
			return Err(format!(
				"the analytics service refused the live connection ({status}): {body}"
			));
		}

		let mut buffer = String::new();
		let mut connection: Option<String> = None;
		let mut since_credit = 0_u32;

		while let Some(chunk) = response
			.chunk()
			.await
			.map_err(|error| format!("the live connection dropped: {error}"))?
		{
			buffer.push_str(&String::from_utf8_lossy(&chunk));
			// Line endings may be `\r\n`; frames end at a blank line.
			if buffer.contains('\r') {
				buffer = buffer.replace("\r\n", "\n");
			}

			while let Some(end) = buffer.find("\n\n") {
				let block: String = buffer.drain(..end + 2).collect();
				let data: Vec<&str> = block
					.lines()
					.filter_map(|line| line.strip_prefix("data:"))
					.map(|line| line.strip_prefix(' ').unwrap_or(line))
					.collect();

				if data.is_empty() {
					continue;
				}

				let Ok(frame) = serde_json::from_str::<ServerFrame>(&data.join("\n")) else {
					continue;
				};

				match frame {
					ServerFrame::Welcome {
						connection: name, ..
					} => {
						let args = match cursor {
							Some(cursor) => serde_json::json!({ "cursor": cursor }),
							None => serde_json::json!({}),
						};
						self.post_frames(
							&name,
							&[
								ClientFrame::Hello { resume: None },
								ClientFrame::Stream {
									stream: STREAM,
									name: "live_events".to_owned(),
									args,
									credits: CREDITS,
								},
							],
						)
						.await?;
						connection = Some(name);
						*self.0.status.lock().unwrap_or_else(PoisonError::into_inner) =
							Status::Live;
					}
					ServerFrame::Item { stream, value, .. } if stream == STREAM => {
						if let Ok(item) = serde_json::from_value::<TailItem>(value) {
							*cursor = Some(item.event.seq);
							self.record(&item);
							let _ = self.0.sender.send(Arc::new(item));
						}

						since_credit += 1;

						if since_credit >= CREDITS / 2
							&& let Some(name) = &connection
						{
							self.post_frames(
								name,
								&[ClientFrame::Credit {
									stream: STREAM,
									n: since_credit,
								}],
							)
							.await?;
							since_credit = 0;
						}
					}
					ServerFrame::End {
						stream,
						reason,
						err,
						..
					} if stream == STREAM => {
						return Err(match err {
							Some(error) => format!("the live tail ended: {}", error.message),
							None => format!("the live tail ended ({reason})"),
						});
					}
					ServerFrame::Reconnect { .. } => return Ok(()),
					_ => {}
				}
			}
		}

		Ok(())
	}
}

/// What a page's `analytics_live` stream sends: the pulse at once and then
/// every second, and with `events`, each event as the hub hears it.
pub(crate) fn follow(connection: super::Connection, events: bool) -> BoxStream<'static, Live> {
	let hub = connection.live().clone();
	let receiver = hub.subscribe();
	let mut tick = tokio::time::interval(Duration::from_secs(1));
	tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
	let state = Following {
		hub,
		receiver,
		tick,
		connection,
		typed: None,
	};

	futures::stream::unfold(state, move |mut state| {
		async move {
			loop {
				tokio::select! {
					_ = state.tick.tick() => {
						let pulse = Live::Pulse(state.hub.pulse().shown());
						return Some((pulse, state));
					}
					item = state.receiver.recv(), if events => match item {
						Ok(item) => {
							let typed = state.typed(&item.event.name).await;
							return Some((Live::Event(arrival(&item, typed)), state));
						}
						Err(RecvError::Lagged(_)) => {}
						Err(RecvError::Closed) => return None,
					},
				}
			}
		}
	})
	.boxed()
}

/// One page's share of the hub.
struct Following {
	hub: Hub,
	receiver: broadcast::Receiver<Arc<TailItem>>,
	tick: tokio::time::Interval,
	connection: super::Connection,
	/// The names of the events the app declared fields for, read once the
	/// first event arrives.
	typed: Option<Vec<String>>,
}

impl Following {
	/// Whether event `name` has declared fields.
	async fn typed(&mut self, name: &str) -> bool {
		if self.typed.is_none() {
			self.typed = Some(typed_names(&self.connection).await);
		}

		self.typed
			.as_ref()
			.is_some_and(|typed| typed.iter().any(|typed| typed == name))
	}
}

/// The names of the events the app declared with `#[derive(Event)]`.
pub(crate) async fn typed_names(connection: &super::Connection) -> Vec<String> {
	connection
		.facts()
		.await
		.map(|facts| {
			facts
				.definitions
				.into_iter()
				.filter(|definition| definition.fields.is_some())
				.map(|definition| definition.name)
				.collect()
		})
		.unwrap_or_default()
}

/// A tail item as the Live page lists it.
fn arrival(item: &TailItem, typed: bool) -> Arrival {
	Arrival {
		seq: item.event.seq,
		name: item.event.name.clone(),
		distinct_id: item.event.distinct_id.clone(),
		timestamp: item.event.timestamp.clone(),
		properties: item.event.properties.clone(),
		typed,
	}
}
