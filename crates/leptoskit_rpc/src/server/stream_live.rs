//! A live server for streams alone: the same socket, the same server-sent
//! events fallback, and the same frames as [`Live`](super::Live), with no
//! database behind it.
//!
//! ```ignore
//! let streams = rpc::server::LiveStreams::new()
//! 	.identify(api_keys)
//! 	.origin("https://myapp.com");
//! let app = Router::new().merge(streams.router());
//! ```
//!
//! It opens every `#[rpc::stream]` whose body takes
//! [`StreamCx`](super::StreamCx), and any client's `use_stream` reads it. A
//! query, a mutation, an action or a channel sent to it is answered with
//! why it can't be served here, so a client that expects the full server
//! shows an error instead of waiting.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;

use axum::Router;
use axum::routing::get;
use axum::routing::post;
use tokio::sync::broadcast;
use tokio::sync::mpsc;

use super::Identify;
use super::Identity;
use super::IdentityLease;
use super::stream::StreamOpen;
use super::stream::StreamSet;
use super::transport::CallerSlot;
use super::transport::FRAME_BURST;
use super::transport::FRAMES_PER_SECOND;
use super::transport::Host;
use super::transport::Incoming;
use super::transport::OUTGOING;
use super::transport::Opened;
use super::transport::Outbound;
use super::transport::STALL;
use super::transport::socket;
use super::transport::sse;
use crate::live::ClientFrame;
use crate::live::HEARTBEAT_SECONDS;
use crate::live::ServerFrame;
use crate::live::WireError;

/// A live server that serves streams and nothing else, for a server with
/// no database. See [the module](self).
///
/// Every caller is anonymous until [`identify`](Self::identify) says
/// otherwise. An identity's
/// [`expires_at`](IdentityLease::expires_at) and
/// [`refresh_at`](IdentityLease::refresh_at) are kept; the rows its lease
/// reads are not watched, since there is no change log to watch them in,
/// so a revoked identity keeps its streams until it expires or reconnects.
#[derive(Clone)]
pub struct LiveStreams {
	inner: Arc<Inner>,
}

struct Inner {
	identify: Arc<dyn Identify>,
	origin: Option<String>,
	service: Option<&'static str>,
	live_path: String,
	stall: Duration,
	shared: Arc<Shared>,
}

/// What every copy of a [`LiveStreams`] shares, whatever its settings.
struct Shared {
	drain: broadcast::Sender<()>,
	connections: AtomicU64,
	events: Mutex<HashMap<String, sse::Handle>>,
}

impl Default for LiveStreams {
	fn default() -> Self {
		Self::new()
	}
}

impl LiveStreams {
	/// A server for the app's streams, at [`/live`](crate::live::LIVE_PATH).
	#[must_use]
	pub fn new() -> Self {
		Self {
			inner: Arc::new(Inner {
				identify: Arc::new(super::Anonymous),
				origin: None,
				service: None,
				live_path: crate::live::LIVE_PATH.to_owned(),
				stall: STALL,
				shared: Arc::new(Shared {
					drain: broadcast::channel(8).0,
					connections: AtomicU64::new(0),
					events: Mutex::new(HashMap::new()),
				}),
			}),
		}
	}

	/// Tells callers apart with `identify`, such as `leptoskit_auth`'s
	/// `Auth` or an API-key check.
	#[must_use]
	pub fn identify(self, identify: impl Identify) -> Self {
		self.identify_arc(Arc::new(identify))
	}

	/// [`identify`](Self::identify) with an already shared implementation.
	#[must_use]
	pub fn identify_arc(self, identify: Arc<dyn Identify>) -> Self {
		self.with(|inner| inner.identify = identify)
	}

	/// The one origin a browser connection may come from, such as
	/// `https://myapp.com`. A native app sends no `Origin` and is allowed
	/// through to authenticate with
	/// [`ClientFrame::Auth`](crate::live::ClientFrame::Auth).
	///
	/// Without it, any origin may connect: development's default, and what
	/// a same-origin app gets from the page's own `Origin`.
	#[must_use]
	pub fn origin(self, origin: impl Into<String>) -> Self {
		let origin = origin.into();
		self.with(|inner| {
			inner.origin = Some(origin.trim_end_matches('/').to_owned());
		})
	}

	/// Serves the streams of the service `name`; `None` serves the app's
	/// own.
	#[must_use]
	pub fn for_service(self, name: Option<&'static str>) -> Self {
		self.with(|inner| inner.service = name)
	}

	/// Mounts the socket where `[rpc]` in `leptoskit.toml` says, as
	/// [`Live::rpc_config`](super::Live::rpc_config) does.
	#[must_use]
	pub fn rpc_config(self, config: &leptoskit_config::RpcConfig) -> Self {
		let path = super::mount_path(&config.resolved_live_path());
		self.with(|inner| inner.live_path = path)
	}

	/// How long a connection may go without taking the next frame (its
	/// client has stopped reading) before it is closed: 30 seconds unless
	/// set.
	#[must_use]
	pub fn stall_timeout(self, timeout: Duration) -> Self {
		self.with(|inner| inner.stall = timeout)
	}

	/// The router serving the live path (the WebSocket,
	/// [`{path}/events`](crate::live::EVENTS_PATH) server-sent events, and
	/// [`{path}/frames`](crate::live::FRAMES_PATH) for their client
	/// frames), relative to wherever it is nested.
	pub fn router<S>(&self) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
	{
		let base = &self.inner.live_path;
		Router::new()
			.route(base, get(socket::upgrade::<Self>))
			.route(&format!("{base}/events"), get(sse::events::<Self>))
			.route(&format!("{base}/frames"), post(sse::frames::<Self>))
			.with_state(self.clone())
	}

	/// Sends every connection
	/// [`ServerFrame::Reconnect`](crate::live::ServerFrame::Reconnect) and
	/// closes it, with jitter so a whole deployment doesn't reconnect at
	/// once. Call it on `SIGTERM`, before the server stops accepting. Each
	/// open stream ends as interrupted, and a resumable one carries on from
	/// its cursor when the client is back.
	pub fn drain(&self) {
		let _ = self.inner.shared.drain.send(());
	}

	/// How many connections this server has opened, over either transport,
	/// including closed ones.
	#[must_use]
	pub fn connections(&self) -> u64 {
		self.inner.shared.connections.load(Ordering::Relaxed)
	}

	fn with(self, change: impl FnOnce(&mut Inner)) -> Self {
		let mut inner = Inner {
			identify: Arc::clone(&self.inner.identify),
			origin: self.inner.origin.clone(),
			service: self.inner.service,
			live_path: self.inner.live_path.clone(),
			stall: self.inner.stall,
			shared: Arc::clone(&self.inner.shared),
		};
		change(&mut inner);
		Self {
			inner: Arc::new(inner),
		}
	}

	/// Runs a connection until its client goes, its transport closes, or
	/// the server drains it.
	async fn run(&self, opened: Opened) {
		let Opened {
			number,
			connection,
			identity,
			mut frames,
			out,
			resume_floor: _,
			caller,
		} = opened;
		let mut session = Session {
			live: self.clone(),
			identity,
			lease: IdentityLease::default(),
			caller,
			out,
			streams: StreamSet::new(OUTGOING),
			frame_tokens: FRAME_BURST * 1_000,
			frame_at: std::time::Instant::now(),
		};

		session.note_caller();
		// There is no change log, so the version never moves.
		session
			.send(ServerFrame::Welcome {
				connection,
				version: 0,
				heartbeat_seconds: HEARTBEAT_SECONDS,
				outdated_client: false,
			})
			.await;

		let mut drained = self.inner.shared.drain.subscribe();

		if session.serve(&mut frames, &mut drained).await && session.interrupt().await {
			let after_ms = 250 + u64::from(number).wrapping_mul(7919) % 2_000;
			session.send(ServerFrame::Reconnect { after_ms }).await;
		}

		session.streams.abort_all();
	}
}

impl std::fmt::Debug for LiveStreams {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("LiveStreams")
			.field("path", &self.inner.live_path)
			.finish_non_exhaustive()
	}
}

impl Host for LiveStreams {
	fn identifier(&self) -> &dyn Identify {
		&*self.inner.identify
	}

	fn allowed_origin(&self) -> Option<&str> {
		self.inner.origin.as_deref()
	}

	fn stall(&self) -> Duration {
		self.inner.stall
	}

	fn next_number(&self) -> u32 {
		let number = self
			.inner
			.shared
			.connections
			.fetch_add(1, Ordering::Relaxed)
			+ 1;
		u32::try_from(number).unwrap_or(u32::MAX)
	}

	fn event_streams(&self) -> MutexGuard<'_, HashMap<String, sse::Handle>> {
		self.inner
			.shared
			.events
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
	}

	fn serve(&self, opened: Opened) -> impl Future<Output = ()> + Send {
		self.run(opened)
	}
}

/// One connection's streams, and who opened them.
struct Session {
	live: LiveStreams,
	identity: Identity,
	lease: IdentityLease,
	caller: CallerSlot,
	out: mpsc::Sender<Outbound>,
	streams: StreamSet,
	/// Frames the connection may still send, as thousandths of a frame,
	/// refilled by elapsed time.
	frame_tokens: u64,
	frame_at: std::time::Instant,
}

impl Session {
	/// Reads frames and answers them, and says whether the server is
	/// draining and wants the client back.
	async fn serve(
		&mut self,
		frames: &mut mpsc::Receiver<Incoming>,
		drained: &mut broadcast::Receiver<()>,
	) -> bool {
		let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECONDS));
		heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

		if !self.validate().await {
			return false;
		}

		heartbeat.tick().await;

		loop {
			let deadline = self.deadline();

			tokio::select! {
				() = wait_until(deadline) => {
					if !self.check_deadline().await { return false; }
				}
				() = self.out.closed() => return false,
				_ = drained.recv() => return true,
				incoming = frames.recv() => {
					let Some(incoming) = incoming else { return false };
					if !self.handle(incoming).await { return false; }
				}
				Some(frame) = self.streams.recv() => {
					if let Some(text) = self.streams.accept(frame)
						&& !self.deliver(Outbound::Frame { text, version: None }).await
					{
						return false;
					}
				}
				_ = heartbeat.tick() => {
					if !self.deliver(Outbound::Heartbeat).await { return false; }
				}
			}
		}
	}

	async fn send(&self, frame: ServerFrame) -> bool {
		let Some(outbound) = Outbound::frame(&frame, None) else {
			return true;
		};
		self.deliver(outbound).await
	}

	/// Queues `outbound` for the transport. A client that stops reading
	/// fills the queue; past the stall timeout the connection closes.
	async fn deliver(&self, outbound: Outbound) -> bool {
		matches!(
			tokio::time::timeout(self.live.inner.stall, self.out.send(outbound)).await,
			Ok(Ok(()))
		)
	}

	async fn done(&self, request: u32, err: WireError) -> bool {
		self.send(ServerFrame::Done {
			request,
			ok: None,
			err: Some(err),
			version: 0,
		})
		.await
	}

	/// Charges one frame against the connection's budget, refilled by
	/// elapsed time.
	fn spend_frame(&mut self) -> bool {
		let now = std::time::Instant::now();
		let elapsed = u64::try_from(now.saturating_duration_since(self.frame_at).as_millis())
			.unwrap_or(u64::MAX);
		self.frame_at = now;
		self.frame_tokens = self
			.frame_tokens
			.saturating_add(elapsed.saturating_mul(FRAMES_PER_SECOND))
			.min(FRAME_BURST * 1_000);

		if self.frame_tokens < 1_000 {
			return false;
		}

		self.frame_tokens -= 1_000;

		true
	}

	/// Handles one frame. Returns whether the connection stays open.
	async fn handle(&mut self, incoming: Incoming) -> bool {
		if !self.spend_frame() {
			self.done(
				0,
				WireError::invalid(
					"this connection is sending frames faster than the server serves them",
				),
			)
			.await;

			return false;
		}

		let frame = match incoming {
			Incoming::Frame(frame) => frame,
			Incoming::Invalid(error) => return self.done(0, error).await,
		};

		match frame {
			// Nothing here has a version to resume from.
			ClientFrame::Hello { .. }
			| ClientFrame::Unknown
			| ClientFrame::Unsub { .. }
			| ClientFrame::Leave { .. }
			| ClientFrame::Signal { .. } => true,
			ClientFrame::Ping => self.send(ServerFrame::Pong).await,
			ClientFrame::Auth { token } => self.authenticate(token).await,
			ClientFrame::Stream {
				stream,
				name,
				args,
				credits,
			} => {
				let cx = StreamOpen::new(self.identity.clone());
				let refused =
					self.streams
						.open(self.live.inner.service, stream, &name, args, credits, cx);

				match refused {
					Some(frame) => self.send(frame).await,
					None => true,
				}
			}
			ClientFrame::Credit { stream, n } => {
				self.streams.credit(stream, n);
				true
			}
			ClientFrame::Cancel { stream } => {
				self.streams.cancel(stream);
				true
			}
			ClientFrame::Sub { id, .. } => self.done(id, streams_only("a live query")).await,
			ClientFrame::Mutate { request, .. } => {
				self.done(request, streams_only("a mutation")).await
			}
			ClientFrame::Act { request, .. } => self.done(request, streams_only("an action")).await,
			ClientFrame::Join { channel, .. } => {
				self.send(ServerFrame::ChannelError {
					channel,
					err: streams_only("a channel"),
				})
				.await
			}
		}
	}

	/// Stops every stream, telling the client each was interrupted.
	async fn interrupt(&mut self) -> bool {
		for text in self.streams.interrupt().await {
			if !self
				.deliver(Outbound::Frame {
					text,
					version: None,
				})
				.await
			{
				return false;
			}
		}

		true
	}

	async fn authenticate(&mut self, token: String) -> bool {
		// A refused token closes the connection, so guessing means paying
		// for a new handshake each time.
		let identity = match self.live.inner.identify.identify_token(&token).await {
			Ok(identity) => identity,
			Err(error) => return self.identity_failure(&error).await,
		};

		if let Some(error) = super::transport::refuse_service(&identity) {
			return self.identity_failure(&error).await;
		}

		self.identity = identity;

		if !self.validate().await {
			return false;
		}

		self.note_caller();

		// A stream runs as whoever opened it, so none outlives the change.
		if !self.interrupt().await {
			return false;
		}

		self.send(ServerFrame::AuthStatus {
			refused: false,
			expiring: false,
		})
		.await
	}

	/// Asks whether the connection may keep its identity, and until when.
	async fn validate(&mut self) -> bool {
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

		self.lease = lease;

		true
	}

	fn deadline(&self) -> Option<SystemTime> {
		self.lease
			.refresh_at
			.into_iter()
			.chain(self.lease.expires_at)
			.min()
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
			// A failed lookup doesn't mean the person signed out.
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
}

/// Why this server can't serve `what`.
fn streams_only(what: &str) -> WireError {
	WireError {
		status: 501,
		message: format!(
			"this live server serves streams only, and this is {what}: serve it with \
			 `rpc::server::Live`, which needs a database"
		),
		code: None,
		retry_after: None,
	}
}

/// Waits until `deadline`, or forever when there is none.
async fn wait_until(deadline: Option<SystemTime>) {
	let Some(deadline) = deadline else {
		return std::future::pending().await;
	};

	// A deadline already past still yields, so the loop makes progress.
	let wait = deadline
		.duration_since(SystemTime::now())
		.unwrap_or(Duration::ZERO);
	tokio::time::sleep(wait).await;
}
