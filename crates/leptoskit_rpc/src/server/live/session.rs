//! One live connection, whatever carries it.
//!
//! A session reads [`Incoming`] client frames from a channel and writes
//! [`Outbound`] server frames to another; the WebSocket and server-sent
//! events routes only move frames between those channels and the network.
//! Everything else is here, the same for both: the welcome, heartbeats,
//! drain, limits, identity, subscriptions, mutations, actions, channels,
//! resuming, and the transitions the hub's batches become.
//!
//! The session is the only writer of its connection's transitions and
//! answers, so a mutation's `done` always goes out before the transition
//! its own write causes, and a subscription's first value before any
//! change to it.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;

use leptoskit_db::Db;
use leptoskit_db::ReadEntry;
use pending::PendingTransition;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::watch;

use super::FRAME_BURST;
use super::FRAMES_PER_SECOND;
use super::Identity;
use super::Incoming;
use super::Invocation;
use super::Live;
use super::Outbound;
use super::hub::Batch;
use super::hub::Changed;
use super::hub::Registration;
use super::hub::ShareKey;
use super::run_query;
use crate::Kind;
use crate::live::ClientFrame;
use crate::live::EndReason;
use crate::live::HEARTBEAT_SECONDS;
use crate::live::MAX_ACTIONS;
use crate::live::MAX_CHANNEL_KEY_BYTES;
use crate::live::MAX_CHANNELS;
use crate::live::MAX_FRAME_BYTES;
use crate::live::MAX_PRESENCE_BYTES;
use crate::live::MAX_STREAMS;
use crate::live::MAX_SUBSCRIPTIONS;
use crate::live::PresenceMember;
use crate::live::Resume;
use crate::live::ServerFrame;
use crate::live::SubscriptionUpdate;
use crate::live::Transition;
use crate::live::Update;
use crate::live::WireError;
use crate::server::IdentityLease;
use crate::server::QueryCx;
use crate::server::channel;
use crate::server::registration_in;
use crate::server::run::recording_commits;
use crate::server::stream;
use crate::server::stream::StreamSink;

mod pending;
#[cfg(all(test, feature = "sqlite"))]
mod recovery_tests;

/// How often a subscription's first run may find a round applied under it
/// before it registers anyway and has the hub run it again.
const REGISTER_ATTEMPTS: usize = 3;

/// The most credits a stream may hold at once, however many the client
/// grants.
const MAX_CREDITS: usize = 1 << 20;

/// Who may post frames to a connection: the caller its identity names,
/// which changes when the connection authenticates.
pub(super) type CallerSlot = Arc<Mutex<Option<String>>>;

/// A connection a transport has opened, for [`Live::serve`].
pub(super) struct Opened {
	/// The connection's number, from [`Live::next_number`].
	pub number: u32,
	/// Its name in `welcome`.
	pub connection: String,
	/// Who opened it.
	pub identity: Identity,
	/// The client's frames.
	pub frames: mpsc::Receiver<Incoming>,
	/// Where the session writes.
	pub out: mpsc::Sender<Outbound>,
	/// A version the transport already knows the client reached, such as
	/// server-sent events' `Last-Event-ID`.
	pub resume_floor: Option<u64>,
	/// Kept current with the identity's caller.
	pub caller: CallerSlot,
}

struct Session {
	live: Live,
	number: u32,
	identity: Identity,
	/// Changes whenever this connection accepts a different authentication.
	epoch: u64,
	lease: IdentityLease,
	db: Db,
	member: String,
	caller: CallerSlot,
	out: mpsc::Sender<Outbound>,
	batches: mpsc::UnboundedReceiver<Batch>,
	finished: mpsc::UnboundedReceiver<Finished>,
	acts: mpsc::UnboundedSender<Finished>,
	/// One permit per action that may run at once.
	actions: Arc<tokio::sync::Semaphore>,
	applied: watch::Receiver<u64>,
	subscriptions: HashMap<u32, Subscription>,
	joined: HashMap<String, Joined>,
	streams: HashMap<u32, Running>,
	/// What running streams send, forwarded in order.
	stream_frames: mpsc::UnboundedReceiver<String>,
	stream_out: mpsc::UnboundedSender<String>,
	/// The version of the last complete transition queued for transport.
	last_sent: u64,
	/// At most one unfinished transition; new batches coalesce into a later
	/// full snapshot instead of growing an unbounded per-connection queue.
	pending: Option<PendingTransition>,
	pending_progress: tokio::time::Instant,
	/// Results changed while an earlier transition was waiting to be sent.
	full_next: bool,
	/// The latest version represented by the session's stored results.
	latest: u64,
	/// Resume errors for queries that have no registered subscription.
	extra_errors: HashMap<u32, WireError>,
	transport_failed: bool,
	/// The newest commit a mutation or action answered with that no
	/// transition has reached yet.
	awaiting: Option<u64>,
	resume_floor: Option<u64>,
	/// Frames the connection may still send, as thousandths of a frame,
	/// refilled by elapsed time.
	frame_tokens: u64,
	frame_at: std::time::Instant,
}

struct Subscription {
	query: String,
	args: Value,
	/// The latest result, including failures; never recover an earlier success
	/// after a policy or identity change makes the current query fail.
	result: Result<Arc<Value>, WireError>,
}

impl Subscription {
	fn apply(&mut self, changed: Changed) -> Update {
		let (update, result) = changed.into_update();
		self.result = result;
		update
	}

	fn full_update(&self) -> Update {
		match &self.result {
			Ok(value) => Update::Ok((**value).clone()),
			Err(error) => Update::Err(error.clone()),
		}
	}
}

struct Joined {
	name: String,
	args: Value,
	state: Value,
	/// The room it shares with the others: see [`super::room_key`].
	room: String,
}

/// A stream the connection opened, and the room its client has left.
struct Running {
	task: tokio::task::JoinHandle<()>,
	credits: Arc<tokio::sync::Semaphore>,
}

/// A mutation or action that finished, and the commit it reached.
struct Finished {
	epoch: u64,
	request: u32,
	result: Result<Value, WireError>,
	version: Option<i64>,
}

/// A subscription's first result, and where it came from.
struct Started {
	value: Arc<Value>,
	reads: Vec<ReadEntry>,
	/// The version the hub had applied when the value was registered: the
	/// value includes every commit up to it.
	at: u64,
}

impl Outbound {
	pub(super) fn frame(frame: &ServerFrame, version: Option<u64>) -> Option<Self> {
		let text = serde_json::to_string(frame).ok()?;
		Some(Self::Frame { text, version })
	}
}

impl Live {
	/// Runs a connection until its client goes, its transport closes, or
	/// the server drains it.
	pub(super) async fn serve(&self, opened: Opened) {
		let Opened {
			number,
			connection,
			identity,
			mut frames,
			out,
			resume_floor,
			caller,
		} = opened;

		if let Err(error) = self.start().await {
			if let Some(frame) = Outbound::frame(
				&ServerFrame::Done {
					request: 0,
					ok: None,
					err: Some(error),
					version: 0,
				},
				None,
			) {
				let _ = out.send(frame).await;
			}

			return;
		}

		let (events, batches) = mpsc::unbounded_channel();
		let (acts, finished) = mpsc::unbounded_channel();
		let (stream_out, stream_frames) = mpsc::unbounded_channel();
		self.hub().connect(number, events);
		self.connections().insert(number, out.clone());
		let version = self.hub().applied();
		let member = identity
			.caller
			.clone()
			.unwrap_or_else(|| format!("live-{number}"));
		let mut session = Session {
			live: self.clone(),
			number,
			db: self.inner.db.with_context(identity.context.clone()),
			identity,
			epoch: 0,
			lease: IdentityLease::default(),
			member,
			caller,
			out,
			batches,
			finished,
			acts,
			actions: Arc::new(tokio::sync::Semaphore::new(MAX_ACTIONS)),
			applied: self.hub().watch(),
			subscriptions: HashMap::new(),
			joined: HashMap::new(),
			streams: HashMap::new(),
			stream_frames,
			stream_out,
			last_sent: version,
			pending: None,
			pending_progress: tokio::time::Instant::now(),
			full_next: false,
			latest: version,
			extra_errors: HashMap::new(),
			transport_failed: false,
			awaiting: None,
			resume_floor,
			frame_tokens: FRAME_BURST * 1_000,
			frame_at: std::time::Instant::now(),
		};
		session.note_caller();

		session
			.send(ServerFrame::Welcome {
				connection,
				version,
				heartbeat_seconds: HEARTBEAT_SECONDS,
				outdated_client: false,
			})
			.await;

		let mut drained = self.inner.shared.drain.subscribe();

		if session.run(&mut frames, &mut drained).await {
			session.interrupt_streams().await;
			let after_ms = 250 + u64::from(number).wrapping_mul(7919) % 2_000;
			session.send(ServerFrame::Reconnect { after_ms }).await;
		}

		for running in session.streams.values() {
			running.task.abort();
		}

		session.leave_all();
		self.hub()
			.disconnect(number, session.subscriptions.keys().copied());
		self.connections().remove(&number);
	}
}

impl Session {
	/// Reads frames and batches, answers them, and says whether the server
	/// is draining and wants the client back.
	async fn run(
		&mut self,
		frames: &mut mpsc::Receiver<Incoming>,
		drained: &mut broadcast::Receiver<()>,
	) -> bool {
		let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECONDS));
		heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

		if !self.validate_identity().await {
			return false;
		}

		heartbeat.tick().await;
		loop {
			if !self.check_due_deadline().await {
				return false;
			}
			if let Some(draining) = self.step(frames, drained, &mut heartbeat).await {
				return draining;
			}

			if !self.settle(false).await {
				return false;
			}
		}
	}

	async fn step(
		&mut self,
		frames: &mut mpsc::Receiver<Incoming>,
		drained: &mut broadcast::Receiver<()>,
		heartbeat: &mut tokio::time::Interval,
	) -> Option<bool> {
		let stall_deadline = self.pending_deadline();
		let deadline = self.next_identity_deadline();
		let sender = self.out.clone();
		tokio::select! {
			// Randomized selection keeps client input and completion fair under continuous writes.
			() = capacity_deadline(stall_deadline) => return Some(false),
			() = identity_deadline(deadline) => {
				if !self.check_deadline().await { return Some(false); }
			}
			() = self.out.closed() => return Some(false),
			_ = drained.recv() => return Some(true),
			permit = sender.reserve_owned(), if self.pending.is_some() || self.full_next => {
				let Ok(permit) = permit else { return Some(false) };
				if !self.send_pending(permit) { return Some(false); }
			}
			Some(batch) = self.batches.recv() => {
				if !self.apply(batch).await { return Some(false); }
			}
			incoming = frames.recv() => {
				let Some(incoming) = incoming else { return Some(false) };
				if !self.handle(incoming).await {
					return Some(false);
				}
			}
			Some(finished) = self.finished.recv() => {
				if !self.answer(finished).await {
					return Some(false);
				}
			}
			Some(text) = self.stream_frames.recv() => {
				if !self.deliver(Outbound::Frame { text, version: None }).await {
					return Some(false);
				}
			}
			Ok(()) = self.applied.changed(), if self.awaiting.is_some() => {}
			_ = heartbeat.tick() => {
				// An idle connection still learns how far the log has
				// gone, so it resumes from somewhere recent.
				if !self.heartbeat().await { return Some(false); }
			}
		}
		None
	}

	fn pending_deadline(&self) -> Option<tokio::time::Instant> {
		let blocked = (self.pending.is_some() || self.full_next) && self.out.capacity() == 0;
		blocked.then(|| self.pending_progress + self.live.stall())
	}

	async fn heartbeat(&mut self) -> bool {
		self.settle(true).await && self.deliver(Outbound::Heartbeat).await
	}

	/// Sends what the hub has queued, and an empty transition when a
	/// mutation is waiting for a commit that changed none of this
	/// connection's results, or when `catch_up` asks for one.
	async fn settle(&mut self, catch_up: bool) -> bool {
		// Return to the select loop after a bounded batch so a continuously
		// busy database cannot starve available transport capacity.
		for _ in 0..super::OUTGOING {
			let Ok(batch) = self.batches.try_recv() else {
				break;
			};
			if !self.apply(batch).await {
				return false;
			}
		}
		if self.transport_failed {
			return false;
		}
		if !self.batches.is_empty() {
			// The stored results have not applied every queued batch yet.
			return true;
		}

		let applied = *self.applied.borrow_and_update();
		let due = catch_up || self.awaiting.is_some_and(|waiting| waiting <= applied);

		if due && self.last_sent < applied {
			self.send_transition(self.last_sent, Vec::new(), applied, false);
		}

		if self
			.awaiting
			.is_some_and(|waiting| waiting <= self.last_sent)
		{
			self.awaiting = None;
		}
		true
	}

	async fn send(&self, frame: ServerFrame) -> bool {
		let Some(outbound) = Outbound::frame(&frame, None) else {
			return true;
		};
		self.deliver(outbound).await
	}

	/// Queues `outbound` for the transport. A client that stops reading
	/// fills the queue; past the stall timeout the connection closes,
	/// rather than holding everything the hub sends it meanwhile.
	async fn deliver(&self, outbound: Outbound) -> bool {
		matches!(
			tokio::time::timeout(self.live.stall(), self.out.send(outbound)).await,
			Ok(Ok(()))
		)
	}

	async fn done(&self, request: u32, result: Result<Value, WireError>) -> bool {
		let (ok, err) = match result {
			Ok(value) => (Some(value), None),
			Err(error) => (None, Some(error)),
		};

		self.send(ServerFrame::Done {
			request,
			ok,
			err,
			version: self.last_sent,
		})
		.await
	}

	/// Charges one frame against the connection's budget, refilled by
	/// elapsed time. Returns whether the frame is served; a connection
	/// that has spent its burst and keeps sending is closed by the caller.
	fn spend_frame(&mut self) -> bool {
		let now = std::time::Instant::now();
		let elapsed = now.saturating_duration_since(self.frame_at).as_millis() as u64;
		self.frame_at = now;
		self.frame_tokens =
			(self.frame_tokens + elapsed * FRAMES_PER_SECOND).min(FRAME_BURST * 1_000);
		if self.frame_tokens < 1_000 {
			return false;
		}
		self.frame_tokens -= 1_000;
		true
	}

	/// Handles one frame. Returns whether the connection stays open.
	async fn handle(&mut self, incoming: Incoming) -> bool {
		// A frame that doesn't parse is answered too, so it pays like any
		// other.
		if !self.spend_frame() {
			self.done(
				0,
				Err(WireError::invalid(
					"this connection is sending frames faster than the server serves them",
				)),
			)
			.await;
			return false;
		}

		let frame = match incoming {
			Incoming::Frame(frame) => frame,
			Incoming::Invalid(error) => return self.done(0, Err(error)).await,
		};

		match frame {
			ClientFrame::Hello { resume } => {
				match resume {
					Some(resume) => self.resume(resume).await,
					None => true,
				}
			}
			ClientFrame::Unknown => true,
			ClientFrame::Ping => self.send(ServerFrame::Pong).await,
			ClientFrame::Sub { id, query, args } => self.subscribe(id, query, args).await,
			ClientFrame::Unsub { id } => {
				if let Some(pending) = &mut self.pending {
					pending.remove(id);
				}
				self.extra_errors.remove(&id);
				self.subscriptions.remove(&id);
				self.live.hub().leave(self.number, id);
				true
			}
			ClientFrame::Join {
				channel,
				name,
				args,
				state,
			} => self.join(channel, &name, args, state).await,
			ClientFrame::Leave { channel } => {
				self.leave(&channel);
				true
			}
			ClientFrame::Signal { channel, message } => {
				self.signal(&channel, message);
				true
			}
			ClientFrame::Mutate {
				request,
				name,
				args,
				key,
			} => self.mutate(request, &name, args, key).await,
			ClientFrame::Act {
				request,
				name,
				args,
			} => self.act(request, &name, args).await,
			ClientFrame::Auth { token } => self.authenticate(token).await,
			ClientFrame::Stream {
				stream,
				name,
				args,
				credits,
			} => self.open_stream(stream, &name, args, credits).await,
			ClientFrame::Credit { stream, n } => {
				self.credit(stream, n);
				true
			}
			ClientFrame::Cancel { stream } => {
				if let Some(running) = self.streams.remove(&stream) {
					running.task.abort();
				}

				true
			}
		}
	}

	/// Opens a stream as the connection's identity, with room for
	/// `credits` items.
	async fn open_stream(&mut self, stream: u32, name: &str, args: Value, credits: u32) -> bool {
		self.streams
			.retain(|_, running| !running.task.is_finished());

		if let Some(previous) = self.streams.remove(&stream) {
			previous.task.abort();
		}

		let Some(registration) = stream::registration_in(self.live.service_name(), name) else {
			let error = WireError::invalid(format!("no stream is named `{name}`"));
			return self.send(EndReason::Failed(error).frame(stream)).await;
		};

		if self.streams.len() >= MAX_STREAMS {
			let error =
				WireError::invalid(format!("a connection may hold {MAX_STREAMS} streams open"));
			return self.send(EndReason::Failed(error).frame(stream)).await;
		}

		let credits = Arc::new(tokio::sync::Semaphore::new(
			usize::try_from(credits)
				.unwrap_or(MAX_CREDITS)
				.min(MAX_CREDITS),
		));
		let sink = StreamSink::new(stream, self.stream_out.clone(), Arc::clone(&credits));
		let cx = QueryCx::new(self.db.clone(), self.identity.clone());
		let task = tokio::spawn((registration.open)(args, cx, sink));
		self.streams.insert(stream, Running { task, credits });
		true
	}

	/// Gives a stream room for `n` more items.
	fn credit(&self, stream: u32, n: u32) {
		let Some(running) = self.streams.get(&stream) else {
			return;
		};
		let room = MAX_CREDITS.saturating_sub(running.credits.available_permits());
		let n = usize::try_from(n).unwrap_or(room).min(room);

		if n > 0 {
			running.credits.add_permits(n);
		}
	}

	/// Stops every stream still running, after sending what they already
	/// produced, and tells the client each was interrupted: the connection
	/// is going, or is about to read as somebody else.
	async fn interrupt_streams(&mut self) {
		let interrupted: Vec<u32> = self
			.streams
			.drain()
			.filter(|(_, running)| {
				running.task.abort();
				!running.task.is_finished()
			})
			.map(|(stream, _)| stream)
			.collect();

		while let Ok(text) = self.stream_frames.try_recv() {
			let _ = self
				.out
				.send(Outbound::Frame {
					text,
					version: None,
				})
				.await;
		}

		for stream in interrupted {
			self.send(EndReason::Interrupted.frame(stream)).await;
		}
	}

	fn over_limit(&self, id: u32) -> Option<WireError> {
		(self.subscriptions.len() >= MAX_SUBSCRIPTIONS && !self.subscriptions.contains_key(&id))
			.then(|| {
				WireError::invalid(format!(
					"a connection may subscribe to {MAX_SUBSCRIPTIONS} queries"
				))
			})
	}

	async fn subscribe(&mut self, id: u32, query: String, args: Value) -> bool {
		if let Some(error) = self.over_limit(id) {
			return self.done(id, Err(error)).await;
		}

		match self.start(id, &query, &args, true).await {
			Ok(started) => {
				let value = (*started.value).clone();
				self.keep(id, query, args, started.value);
				self.done(id, Ok(value)).await
			}
			Err(error) => self.done(id, Err(error)).await,
		}
	}

	fn keep(&mut self, id: u32, query: String, args: Value, value: Arc<Value>) {
		if let Some(pending) = &mut self.pending {
			pending.remove(id);
		}
		self.extra_errors.remove(&id);
		self.live
			.stats_ref()
			.subscriptions
			.fetch_add(1, Ordering::Relaxed);
		self.subscriptions.insert(
			id,
			Subscription {
				query,
				args,
				result: Ok(value),
			},
		);
	}

	/// Subscribes `id` through the hub: joins the group already watching
	/// the same query as the same caller, or runs it and starts one.
	/// `known` says whether the client will have the value this returns.
	async fn start(
		&self,
		id: u32,
		query: &str,
		args: &Value,
		known: bool,
	) -> Result<Started, WireError> {
		let hub = self.live.hub();
		let key = ShareKey::new(query, args, &self.identity, self.number, id)
			.with_revision(self.lease.continuity.clone());

		// A group of its own is only ever this subscription's earlier one,
		// which may have been to another query: run it again.
		if !matches!(key, ShareKey::Alone { .. })
			&& let Some((_, reads)) = hub.peek(&key)
		{
			let at = hub.applied();

			if let Some(value) = hub.join(self.number, id, &key, known) {
				return Ok(Started { value, reads, at });
			}
		}

		for attempt in 0..=REGISTER_ATTEMPTS {
			let since = hub.applied();
			let (value, reads) = run_query(
				self.live.service_name(),
				query,
				args.clone(),
				self.db.clone(),
				self.identity.clone(),
			)
			.await?;
			let value = Arc::new(value);
			let registration = Registration {
				key: key.clone(),
				service: self.live.service_name(),
				query: query.to_owned(),
				args: args.clone(),
				identity: self.identity.clone(),
				db: self.db.clone(),
				result: Arc::clone(&value),
				reads: reads.clone(),
			};
			let checked = (attempt < REGISTER_ATTEMPTS).then_some(since);

			if hub.register(self.number, id, checked, known, registration) {
				return Ok(Started {
					value,
					reads,
					at: since,
				});
			}
		}

		unreachable!("the last attempt registers whatever the hub has applied")
	}

	/// Subscribes again to what a reconnecting client holds, and sends one
	/// transition with only what changed since the version it reached, or
	/// with everything when the change log no longer reaches back that far.
	async fn resume(&mut self, resume: Resume) -> bool {
		if resume.queries.is_empty() {
			return true;
		}

		let hub = self.live.hub();
		let from = resume.version.max(self.resume_floor.unwrap_or(0));
		let to = hub.applied();
		// What changed after `from`, through `covered`.
		let mut seen_changes = hub.changes_between(from, to).await;
		let full = seen_changes.is_none();
		let mut covered = to;
		let dialect = hub.dialect();

		if !full {
			self.live
				.stats_ref()
				.resumes
				.fetch_add(1, Ordering::Relaxed);
		}

		let mut updates = Vec::new();
		// One subscription per ID, as the client holds them: repeats would
		// each run a query for the price of one frame, past the
		// subscription limit, which counts IDs.
		let mut ids = HashSet::new();
		let mut queries = resume.queries;
		queries.reverse();
		queries.retain(|query| ids.insert(query.id));
		queries.reverse();

		for query in queries {
			if let Some(error) = self.over_limit(query.id) {
				updates.push(SubscriptionUpdate {
					id: query.id,
					update: Update::Err(error),
				});
				continue;
			}

			match self.start(query.id, &query.query, &query.args, false).await {
				Ok(started) => {
					// A round applied while the client resumed: read what it
					// had, too.
					if started.at > covered
						&& let Some(seen) = &mut seen_changes
					{
						match self.live.hub().changes_between(covered, started.at).await {
							Some(more) => {
								seen.extend(more);
								covered = started.at;
							}
							None => seen_changes = None,
						}
					}

					let changed = match &seen_changes {
						Some(changes) if started.at <= covered => {
							started.reads.is_empty()
								|| started.reads.iter().any(|read| {
									changes
										.iter()
										.any(|change| read.could_change(change, dialect))
								})
						}
						_ => true,
					};

					if changed {
						self.live.hub().known(self.number, query.id, &started.value);
						updates.push(SubscriptionUpdate {
							id: query.id,
							update: Update::Ok((*started.value).clone()),
						});
					}

					self.keep(query.id, query.query, query.args, started.value);
				}
				Err(error) => {
					updates.push(SubscriptionUpdate {
						id: query.id,
						update: Update::Err(error),
					});
				}
			}
		}

		// The transition claims the version the resume started from, even
		// where a result includes later commits: claiming less is safe, and
		// the hub's next batches bring the rest.
		let to = to.max(self.last_sent);
		self.send_transition(from, updates, to, full);
		true
	}

	async fn mutate(&mut self, request: u32, name: &str, args: Value, key: Option<String>) -> bool {
		let Some(registration) = registration_in(self.live.service_name(), name) else {
			return self
				.done(
					request,
					Err(WireError::invalid(format!(
						"no operation is named `{name}`"
					))),
				)
				.await;
		};

		if registration.kind != Kind::Mutation {
			return self
				.done(
					request,
					Err(WireError::invalid(format!(
						"`{name}` is a {}, and a live connection sends only a mutation with \
						 `mutate`",
						registration.kind
					))),
				)
				.await;
		}

		let (result, version) = recording_commits((registration.handler)(Invocation {
			db: self.db.clone(),
			identity: self.identity.clone(),
			request_id: key,
			arguments: args,
			reads: None,
		}))
		.await;

		self.answer(Finished {
			epoch: self.epoch,
			request,
			result: result.map_err(|error| WireError::new(&error)),
			version,
		})
		.await
	}

	/// Runs an action in its own task, so a slow one doesn't hold up the
	/// connection's other frames; its answer comes back to the session.
	async fn act(&mut self, request: u32, name: &str, args: Value) -> bool {
		let Some(registration) = registration_in(self.live.service_name(), name) else {
			return self
				.done(
					request,
					Err(WireError::invalid(format!(
						"no operation is named `{name}`"
					))),
				)
				.await;
		};

		if registration.kind != Kind::Action {
			return self
				.done(
					request,
					Err(WireError::invalid(format!(
						"`{name}` is a {}, and a live connection runs only an action with `act`",
						registration.kind
					))),
				)
				.await;
		}

		// Actions run beside the connection, so each frame could otherwise
		// start one more.
		let Ok(permit) = Arc::clone(&self.actions).try_acquire_owned() else {
			return self
				.done(
					request,
					Err(WireError {
						status: 429,
						message: format!(
							"a connection may run {MAX_ACTIONS} actions at once; wait for one to \
							 finish"
						),
					}),
				)
				.await;
		};
		let invocation = Invocation {
			db: self.db.clone(),
			identity: self.identity.clone(),
			request_id: None,
			arguments: args,
			reads: None,
		};
		let handler = registration.handler;
		let finished = self.acts.clone();
		let epoch = self.epoch;
		tokio::spawn(async move {
			let (result, version) = recording_commits(handler(invocation)).await;
			drop(permit);
			let _ = finished.send(Finished {
				epoch,
				request,
				result: result.map_err(|error| WireError::new(&error)),
				version,
			});
		});
		true
	}

	/// Answers a mutation or an action with the version its writes
	/// committed at, and waits for a transition to reach it.
	async fn answer(&mut self, finished: Finished) -> bool {
		if finished.epoch != self.epoch {
			return self
				.done(
					finished.request,
					Err(WireError {
						status: 401,
						message: "the identity changed while the action ran".to_owned(),
					}),
				)
				.await;
		}
		let version = match finished
			.version
			.and_then(|version| u64::try_from(version).ok())
		{
			Some(version) => {
				if version > self.last_sent {
					self.awaiting = Some(self.awaiting.map_or(version, |seen| seen.max(version)));
				}

				version
			}
			None => self.last_sent,
		};
		let (ok, err) = match finished.result {
			Ok(value) => (Some(value), None),
			Err(error) => (None, Some(error)),
		};

		self.send(ServerFrame::Done {
			request: finished.request,
			ok,
			err,
			version,
		})
		.await
	}

	async fn authenticate(&mut self, token: String) -> bool {
		// A refused token closes the connection, so guessing means paying
		// for a new handshake each time.
		let identity = match self.live.inner.identify.identify_token(&token).await {
			Ok(identity) => identity,
			Err(error) => return self.identity_failure(&error).await,
		};

		// A service reads the app's data by calling its operations, never by
		// watching them: refused here as at the upgrade, so a connection
		// can't open anonymously and then sign in as a service.
		if crate::server::is_service(&identity) {
			return self
				.identity_failure(&crate::Error::forbidden(
					"a service can't hold a live connection to the app: call its operations \
					 instead",
				))
				.await;
		}

		self.pending = None;
		self.pending_progress = tokio::time::Instant::now();
		self.full_next = !self.subscriptions.is_empty();
		self.extra_errors.clear();
		// Detach every old group before awaiting any new identity's reads.
		// Batches already queued belong to the old identity as well.
		for id in self.subscriptions.keys().copied() {
			self.live.hub().leave(self.number, id);
		}
		while self.batches.try_recv().is_ok() {}
		let previous = self.lease.continuity.clone().filter(|key| !key.is_empty());
		self.identity = identity;
		if !self.validate_identity().await {
			return false;
		}
		if previous.is_none() || previous != self.lease.continuity {
			self.epoch = self.epoch.wrapping_add(1);
		}
		self.db = self
			.live
			.inner
			.db
			.with_context(self.identity.context.clone());
		self.note_caller();
		// A stream reads as whoever opened it, so none outlives the change.
		self.interrupt_streams().await;

		// Every result is now read as somebody else: subscribe them all
		// again, under the new identity's own groups.
		let to = self.live.hub().applied();
		let ids: Vec<u32> = self.subscriptions.keys().copied().collect();
		let mut updates = Vec::new();

		for id in ids {
			let Some((query, args)) = self
				.subscriptions
				.get(&id)
				.map(|subscription| (subscription.query.clone(), subscription.args.clone()))
			else {
				continue;
			};
			self.live.hub().leave(self.number, id);

			match self.start(id, &query, &args, true).await {
				Ok(started) => {
					updates.push(SubscriptionUpdate {
						id,
						update: Update::Ok((*started.value).clone()),
					});

					if let Some(subscription) = self.subscriptions.get_mut(&id) {
						subscription.result = Ok(started.value);
					}
				}
				Err(error) => {
					if let Some(subscription) = self.subscriptions.get_mut(&id) {
						subscription.result = Err(error.clone());
					}
					updates.push(SubscriptionUpdate {
						id,
						update: Update::Err(error),
					});
				}
			}
		}

		if !updates.is_empty() {
			let to = to.max(self.last_sent);
			self.send_transition(self.last_sent, updates, to, true);
		}

		// The identity changed, so every channel is checked again.
		self.member = self
			.identity
			.caller
			.clone()
			.unwrap_or_else(|| format!("live-{}", self.number));
		self.reauthorize().await;

		self.send(ServerFrame::AuthStatus {
			refused: false,
			expiring: false,
		})
		.await
	}

	/// Validate on the primary and install the dependencies without losing
	/// a commit that landed while the database was being read.
	async fn validate_identity(&mut self) -> bool {
		for _ in 0..REGISTER_ATTEMPTS {
			let since = self.live.hub().applied();
			let lease = match self.live.inner.identify.lease(&self.identity).await {
				Ok(lease) => lease,
				Err(error) => return self.identity_failure(&error).await,
			};
			if lease
				.expires_at
				.is_some_and(|expires| expires <= SystemTime::now())
			{
				return self.refuse_identity().await;
			}
			if self
				.live
				.hub()
				.watch_identity(self.number, lease.reads.clone(), since)
			{
				self.lease = lease;
				return true;
			}
		}
		// A busy log may prevent registration even when the identity is
		// valid. Reconnect instead of telling the app it was signed out.
		self.send(ServerFrame::Reconnect { after_ms: 250 }).await;
		false
	}

	fn next_identity_deadline(&self) -> Option<SystemTime> {
		self.lease
			.refresh_at
			.into_iter()
			.chain(self.lease.expires_at)
			.min()
	}

	async fn check_due_deadline(&mut self) -> bool {
		let deadline = self.next_identity_deadline();
		if deadline.is_some_and(|at| at <= SystemTime::now()) {
			self.check_deadline().await
		} else {
			true
		}
	}

	async fn check_deadline(&mut self) -> bool {
		if self
			.lease
			.expires_at
			.is_some_and(|expires| expires <= SystemTime::now())
		{
			return self.refuse_identity().await;
		}
		self.lease.refresh_at = None;
		self.send(ServerFrame::AuthStatus {
			refused: false,
			expiring: true,
		})
		.await
	}

	async fn identity_failure(&self, error: &crate::Error) -> bool {
		if matches!(error.status(), 401 | 403) {
			self.refuse_identity().await
		} else {
			// A failed database read doesn't mean the person signed out.
			self.send(ServerFrame::Reconnect { after_ms: 500 }).await;
			false
		}
	}

	async fn refuse_identity(&self) -> bool {
		self.send(ServerFrame::AuthStatus {
			refused: true,
			expiring: false,
		})
		.await;
		false
	}

	/// Records who may post frames to this connection.
	fn note_caller(&self) {
		self.caller
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.clone_from(&self.identity.caller);
	}

	/// Turns one of the hub's batches into a transition.
	async fn apply(&mut self, batch: Batch) -> bool {
		if self
			.lease
			.expires_at
			.is_some_and(|expires| expires <= SystemTime::now())
		{
			return self.refuse_identity().await;
		}
		if batch.revalidate && !self.validate_identity().await {
			return false;
		}
		let mut updates = Vec::new();

		for (id, changed) in batch.changes {
			let Some(subscription) = self.subscriptions.get_mut(&id) else {
				// Unsubscribed while the batch was on its way.
				continue;
			};
			let update = subscription.apply(changed);
			updates.push(SubscriptionUpdate { id, update });
		}

		let to = batch.to.max(self.last_sent);
		self.send_transition(self.last_sent, updates, to, batch.full);
		true
	}

	/// Retain one unfinished transition and coalesce subsequent changes into
	/// the current subscription results. Capacity wakes the session even if
	/// no further client frame or database commit arrives.
	fn send_transition(
		&mut self,
		from: u64,
		updates: Vec<SubscriptionUpdate>,
		to: u64,
		full: bool,
	) {
		self.latest = self.latest.max(to);
		self.live
			.stats_ref()
			.transitions
			.fetch_add(1, Ordering::Relaxed);
		if self.pending.is_some() || self.full_next {
			self.full_next = true;
			for update in updates {
				if !self.subscriptions.contains_key(&update.id)
					&& let Update::Err(error) = update.update
				{
					if self.extra_errors.len() >= MAX_SUBSCRIPTIONS
						&& !self.extra_errors.contains_key(&update.id)
					{
						self.transport_failed = true;
						return;
					}
					self.extra_errors.insert(update.id, error);
				}
			}
		} else {
			self.pending_progress = tokio::time::Instant::now();
			self.pending = Some(PendingTransition::new(from, updates, to, full));
		}
		// Fill only immediately available slots; never wait inside a batch.
		for _ in 0..super::OUTGOING {
			if self.pending.is_none() && !self.full_next {
				break;
			}
			let Ok(permit) = self.out.clone().try_reserve_owned() else {
				break;
			};
			if !self.send_pending(permit) {
				self.transport_failed = true;
				break;
			}
		}
	}

	fn send_pending(&mut self, permit: mpsc::OwnedPermit<Outbound>) -> bool {
		if self.pending.is_none() && self.full_next {
			let mut updates: Vec<_> = self
				.subscriptions
				.iter()
				.map(|(id, subscription)| {
					SubscriptionUpdate {
						id: *id,
						update: subscription.full_update(),
					}
				})
				.collect();
			updates.extend(self.extra_errors.drain().map(|(id, error)| {
				SubscriptionUpdate {
					id,
					update: Update::Err(error),
				}
			}));
			self.pending = Some(PendingTransition::new(
				self.last_sent,
				updates,
				self.latest,
				true,
			));
			self.full_next = false;
		}
		let Some(pending) = &mut self.pending else {
			return true;
		};
		let Some((frame, complete)) = pending.frame() else {
			return false;
		};
		let to = pending.to;
		permit.send(frame);
		self.pending_progress = tokio::time::Instant::now();
		if complete {
			self.last_sent = to;
			self.pending = None;
		}
		true
	}

	/// Checks every joined channel again, for the new identity.
	async fn reauthorize(&mut self) {
		let keys: Vec<String> = self.joined.keys().cloned().collect();

		for key in keys {
			let Some((name, args, state, room)) = self.joined.get(&key).map(|joined| {
				(
					joined.name.clone(),
					joined.args.clone(),
					joined.state.clone(),
					joined.room.clone(),
				)
			}) else {
				continue;
			};
			let Some(registration) = channel::registration_in(self.live.service_name(), &name)
			else {
				self.leave(&key);
				continue;
			};
			let cx = QueryCx::new(self.db.clone(), self.identity.clone());

			match (registration.authorize)(args, cx).await {
				Ok(()) => {
					self.refresh_member(&room, state);
					self.broadcast_presence(&room);
				}
				Err(error) => {
					self.leave(&key);
					self.channel_error(&key, WireError::new(&error)).await;
				}
			}
		}
	}

	/// Records the new identity in the channel's room.
	fn refresh_member(&mut self, key: &str, state: Value) {
		let mut rooms = self.live.rooms();

		if let Some(room) = rooms.get_mut(key)
			&& let Some(member) = room.members.get_mut(&self.number)
		{
			member.id.clone_from(&self.member);
			member.state = state;
		}
	}

	async fn join(&mut self, channel: String, name: &str, args: Value, state: Value) -> bool {
		if self.joined.contains_key(&channel) {
			self.leave(&channel);
		}

		let Some(registration) = channel::registration_in(self.live.service_name(), name) else {
			return self
				.channel_error(
					&channel,
					WireError::invalid(format!("no channel is named `{name}`")),
				)
				.await;
		};

		// The key names the room, so it must be the arguments that are
		// authorized: anything else would let a member of one instance into
		// another by naming it.
		if serde_json::from_str::<Value>(&channel).ok().as_ref() != Some(&args) {
			return self
				.channel_error(
					&channel,
					WireError::invalid("a channel's key must be its join arguments, serialized"),
				)
				.await;
		}

		if channel.len() > MAX_CHANNEL_KEY_BYTES {
			return self
				.channel_error(
					&channel,
					WireError::invalid(format!(
						"a channel's join arguments may be at most {MAX_CHANNEL_KEY_BYTES} bytes \
						 of JSON"
					)),
				)
				.await;
		}

		if self.joined.len() >= MAX_CHANNELS {
			return self
				.channel_error(
					&channel,
					WireError::invalid(format!("a connection may be in {MAX_CHANNELS} channels")),
				)
				.await;
		}

		if serde_json::to_vec(&state).map_or(true, |bytes| bytes.len() > MAX_PRESENCE_BYTES) {
			return self
				.channel_error(
					&channel,
					WireError::invalid(format!(
						"a presence state may be at most {MAX_PRESENCE_BYTES} bytes of JSON"
					)),
				)
				.await;
		}

		let cx = QueryCx::new(self.db.clone(), self.identity.clone());

		if let Err(error) = (registration.authorize)(args.clone(), cx).await {
			return self.channel_error(&channel, WireError::new(&error)).await;
		}

		let room_key = super::room_key(name, &channel);

		{
			let mut rooms = self.live.rooms();
			let room = rooms.entry(room_key.clone()).or_default();
			room.members.insert(
				self.number,
				PresenceMember {
					id: self.member.clone(),
					state: state.clone(),
				},
			);
		}

		self.joined.insert(
			channel.clone(),
			Joined {
				name: name.to_owned(),
				args,
				state,
				room: room_key.clone(),
			},
		);
		self.broadcast_presence(&room_key);
		true
	}

	fn leave(&mut self, channel: &str) {
		let Some(Joined { room: key, .. }) = self.joined.remove(channel) else {
			return;
		};

		let empty = {
			let mut rooms = self.live.rooms();
			if let Some(room) = rooms.get_mut(&key) {
				room.members.remove(&self.number);
			}

			rooms.get(&key).is_none_or(|room| room.members.is_empty())
		};

		if empty {
			self.live.rooms().remove(&key);
		}

		self.broadcast_presence(&key);
	}

	fn leave_all(&mut self) {
		let keys: Vec<String> = self.joined.keys().cloned().collect();

		for key in keys {
			self.leave(&key);
		}
	}

	fn signal(&self, channel: &str, message: Value) {
		let Some(Joined { room: key, .. }) = self.joined.get(channel) else {
			return;
		};

		let (senders, from) = {
			let rooms = self.live.rooms();
			let Some(room) = rooms.get(key) else {
				return;
			};
			let connections = self.live.connections();
			let senders: Vec<mpsc::Sender<Outbound>> = room
				.members
				.keys()
				.filter(|id| **id != self.number)
				.filter_map(|id| connections.get(id).cloned())
				.collect();
			let from = room
				.members
				.get(&self.number)
				.map(|member| member.id.clone());
			(senders, from)
		};
		self.live.signal_elsewhere(key, from.clone(), &message);
		let frame = ServerFrame::Signal {
			channel: channel.to_owned(),
			from,
			message,
		};
		let Some(outbound) = Outbound::frame(&frame, None) else {
			return;
		};

		for sender in senders {
			let _ = sender.try_send(outbound.clone());
		}
	}

	fn broadcast_presence(&self, channel: &str) {
		self.live.presence_changed(channel);
	}

	async fn channel_error(&self, channel: &str, err: WireError) -> bool {
		self.send(ServerFrame::ChannelError {
			channel: channel.to_owned(),
			err,
		})
		.await
	}
}

async fn capacity_deadline(deadline: Option<tokio::time::Instant>) {
	match deadline {
		Some(at) => tokio::time::sleep_until(at).await,
		None => std::future::pending().await,
	}
}

/// Sleep until a lease deadline, or stay pending for an unlimited identity.
async fn identity_deadline(deadline: Option<SystemTime>) {
	match deadline {
		Some(deadline) => {
			tokio::time::sleep(
				deadline
					.duration_since(SystemTime::now())
					.unwrap_or_default(),
			)
			.await;
		}
		None => std::future::pending().await,
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;
	use crate::live::Patch;
	use crate::live::Upsert;

	fn subscription() -> Subscription {
		Subscription {
			query: "notes".to_owned(),
			args: json!({}),
			result: Ok(Arc::new(json!([{"id": "1", "body": "before"}]))),
		}
	}

	#[test]
	fn full_recovery_keeps_a_query_failure_until_a_new_success() {
		let mut subscription = subscription();
		let error = WireError {
			status: 403,
			message: "no longer available".to_owned(),
		};
		assert_eq!(
			subscription.apply(Changed::Failed(error.clone())),
			Update::Err(error.clone())
		);
		assert_eq!(subscription.full_update(), Update::Err(error));

		let current = json!([{"id": "2", "body": "after"}]);
		subscription.apply(Changed::Whole(Arc::new(current.clone())));
		assert_eq!(subscription.full_update(), Update::Ok(current));
	}

	#[test]
	fn full_recovery_uses_the_complete_value_behind_a_patch() {
		let mut subscription = subscription();
		let row = json!({"id": "1", "body": "after"});
		let value = json!([row.clone()]);
		let patch = Patch {
			upsert: vec![Upsert { at: 0, value: row }],
			..Patch::default()
		};
		assert_eq!(
			subscription.apply(Changed::Patch(
				Arc::new(patch.clone()),
				Arc::new(value.clone())
			)),
			Update::Patch(patch)
		);
		assert_eq!(subscription.full_update(), Update::Ok(value));
	}

	#[test]
	fn full_recovery_preserves_the_latest_error_including_repeated_failures() {
		let mut subscription = subscription();
		for status in [503, 401, 403] {
			let error = WireError {
				status,
				message: format!("failure {status}"),
			};
			subscription.apply(Changed::Failed(error.clone()));
			assert_eq!(subscription.full_update(), Update::Err(error));
		}
	}
}
