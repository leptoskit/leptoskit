//! The streams a server knows, and how one runs: its items go out only as
//! fast as the client grants credits, so the server never reads ahead of
//! what the client has room for.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use futures::FutureExt;
use futures::Stream;
use futures::StreamExt;
use leptoskit_query::BoxFuture;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::sync::mpsc;

use super::Identity;
use crate::Error;
use crate::Result;
use crate::StreamOperation;
use crate::live::EndReason;
use crate::live::MAX_FRAME_BYTES;
use crate::live::MAX_STREAMS;
use crate::live::ServerFrame;
use crate::live::WireError;
use crate::stream::Resumable;

#[cfg(test)]
mod tests;

/// One `#[rpc::stream]`, as a server build registers it.
pub struct StreamRegistration {
	/// The stream's name.
	pub name: &'static str,
	/// Whether a reconnecting client opens it again from its cursor.
	pub resumable: bool,
	/// Decodes the arguments, runs the body as the connection's identity,
	/// and sends its items through the sink until it ends.
	#[doc(hidden)]
	pub open: Open,
	/// The service whose live socket serves it; `None` for the app's own.
	pub service: Option<&'static str>,
}

/// What a registration runs.
pub type Open = fn(Value, StreamOpen, StreamSink) -> BoxFuture<'static, ()>;

inventory::collect!(StreamRegistration);

/// The app's stream called `name`: never a service's. See
/// [`registration_in`].
#[must_use]
pub fn registration(name: &str) -> Option<&'static StreamRegistration> {
	registration_in(None, name)
}

/// The stream called `name` that `service`'s live socket serves (`None`
/// for the app's own).
#[must_use]
pub fn registration_in(service: Option<&str>, name: &str) -> Option<&'static StreamRegistration> {
	inventory::iter::<StreamRegistration>()
		.find(|registration| registration.service == service && registration.name == name)
}

/// Decodes the arguments of `S` and pumps it: the `open` a
/// `#[rpc::stream]` registers.
#[doc(hidden)]
pub fn open<S: StreamOperation>(
	args: Value,
	cx: StreamOpen,
	sink: StreamSink,
) -> BoxFuture<'static, ()> {
	Box::pin(async move {
		match serde_json::from_value::<S>(args) {
			Ok(op) => op.pump(cx, sink).await,
			Err(error) => {
				sink.fail(&Error::invalid(format!(
					"the arguments of `{}` don't fit: {error}",
					S::NAME
				)));
			}
		}
	})
}

/// What a `#[rpc::stream]` body receives when it reads nothing from the
/// database: who is calling.
///
/// A stream that takes it runs on any live server, including
/// [`LiveStreams`](super::LiveStreams), which has no database at all. A
/// stream that reads the database as the caller takes
/// [`QueryCx`](crate::QueryCx) instead.
///
/// ```ignore
/// /// An import's progress, a step at a time.
/// #[rpc::stream]
/// pub async fn progress(cx: &rpc::StreamCx, id: String) -> rpc::Result<impl Stream<Item = Step>> {
/// 	let me = cx.require::<SignedIn>()?;
/// 	Ok(imports::watch(me.account, id))
/// }
/// ```
#[derive(Clone, Debug)]
pub struct StreamCx {
	identity: Identity,
}

impl StreamCx {
	/// A context for a stream opened by `identity`.
	#[must_use]
	pub fn new(identity: Identity) -> Self {
		Self { identity }
	}

	/// Who is calling.
	#[must_use]
	pub fn identity(&self) -> &Identity {
		&self.identity
	}

	/// What the server's [`Identify`](super::Identify) said about the
	/// caller.
	#[must_use]
	pub fn context(&self) -> &leptoskit_db::Context {
		&self.identity.context
	}

	/// The caller's global of type `G`, such as the signed-in person.
	#[must_use]
	pub fn get<G: 'static>(&self) -> Option<&G> {
		self.identity.context.get()
	}

	/// The caller's global of type `G`, or [`Error::Unauthorized`] when
	/// there is none: `cx.require::<SignedIn>()?` for a stream only
	/// signed-in people may open.
	///
	/// # Errors
	///
	/// [`Error::Unauthorized`] when the caller has no `G`.
	pub fn require<G: 'static>(&self) -> Result<&G> {
		self.get()
			.ok_or_else(|| Error::unauthorized("sign in to do this"))
	}

	/// Whether the caller is signed in: the request's context has an
	/// [`Actor`](leptoskit_db::Actor), which `leptoskit_auth` adds for a
	/// signed-in session.
	#[must_use]
	pub fn signed_in(&self) -> bool {
		self.identity.context.actor().is_some()
	}
}

/// What a live session hands a stream it opens: the caller, and the
/// database when the server has one. The body's own context is made from
/// it, by the code `#[rpc::stream]` generates.
#[doc(hidden)]
pub struct StreamOpen {
	identity: Identity,
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	db: Option<leptoskit_db::Db>,
}

impl StreamOpen {
	/// A stream opened by `identity` on a server without a database.
	#[must_use]
	pub fn new(identity: Identity) -> Self {
		Self {
			identity,
			#[cfg(any(feature = "postgres", feature = "sqlite"))]
			db: None,
		}
	}

	/// A stream opened by `identity`, reading `db`, which should already
	/// act for them.
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	#[must_use]
	pub fn on(db: leptoskit_db::Db, identity: Identity) -> Self {
		Self {
			identity,
			db: Some(db),
		}
	}

	/// The context of a body that takes [`StreamCx`].
	#[must_use]
	pub fn stream_cx(&self) -> StreamCx {
		StreamCx::new(self.identity.clone())
	}

	/// The context of a body that takes [`QueryCx`](crate::QueryCx), or
	/// reads it through [`rpc::cx()`](crate::cx).
	///
	/// # Errors
	///
	/// When the server that opened the stream has no database.
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	pub fn query_cx(&self) -> Result<super::QueryCx> {
		let Some(db) = &self.db else {
			return Err(Error::server(
				"this stream reads the database, and the live server it was opened on serves \
				 streams only: serve it with `rpc::server::Live`, or take `rpc::StreamCx`",
			));
		};

		Ok(super::QueryCx::new(db.clone(), self.identity.clone()))
	}
}

/// Where a running stream sends its items: the connection's session, gated
/// by the credits its client granted.
#[derive(Debug)]
#[cfg_attr(
	not(feature = "live-streams"),
	allow(dead_code, reason = "only live sessions consume queued frames")
)]
pub(crate) struct Frame {
	pub(crate) stream: u32,
	pub(crate) generation: Arc<()>,
	pub(crate) text: String,
	pub(crate) terminal: bool,
}

/// A producer sink with credit and bounded-output backpressure.
pub struct StreamSink {
	pub(crate) generation: Arc<()>,
	terminal: Arc<AtomicBool>,
	stream: u32,
	/// Credits bound demand, while this bounded queue limits retention even
	/// if a client renews credits before consuming its earlier items.
	out: mpsc::Sender<Frame>,
	credits: Arc<Semaphore>,
	/// An older generated pump calls `fail` synchronously. The owning
	/// session flushes this failure before releasing the stream's task.
	failure: Arc<Mutex<Option<WireError>>>,
}

/// An item, encoded: its value, and where a resumable stream reached.
#[doc(hidden)]
pub type Encoded = Result<(Value, Option<Value>)>;

impl StreamSink {
	#[cfg_attr(
		not(feature = "live-streams"),
		allow(dead_code, reason = "only a live session opens streams")
	)]
	pub(crate) fn new(stream: u32, out: mpsc::Sender<Frame>, credits: Arc<Semaphore>) -> Self {
		Self {
			generation: Arc::new(()),
			terminal: Arc::new(AtomicBool::new(false)),
			stream,
			out,
			credits,
			failure: Arc::new(Mutex::new(None)),
		}
	}

	/// Create this before running the operation and await it afterwards.
	/// Waiting belongs to the tracked producer task, never a detached task.
	#[cfg_attr(
		not(feature = "live-streams"),
		allow(dead_code, reason = "only a live session opens streams")
	)]
	pub(crate) fn completion(&self) -> BoxFuture<'static, ()> {
		let sink = Self {
			generation: Arc::clone(&self.generation),
			terminal: Arc::clone(&self.terminal),
			stream: self.stream,
			out: self.out.clone(),
			credits: Arc::clone(&self.credits),
			failure: Arc::clone(&self.failure),
		};
		Box::pin(async move {
			let error = sink
				.failure
				.lock()
				.unwrap_or_else(PoisonError::into_inner)
				.take();
			if !sink.terminal.load(Ordering::Acquire) {
				sink.end(error.map_or(EndReason::Done, EndReason::Failed))
					.await;
			}
		})
	}

	/// Runs even manually registered producers under the same terminal
	/// lifecycle.
	#[cfg_attr(
		not(feature = "live-streams"),
		allow(dead_code, reason = "only live sessions supervise producers")
	)]
	pub(crate) async fn supervise(self, open: Open, args: Value, cx: StreamOpen) {
		let completion = self.completion();
		let failure = Arc::clone(&self.failure);
		let pump = async move { open(args, cx, self).await };

		if std::panic::AssertUnwindSafe(pump)
			.catch_unwind()
			.await
			.is_err()
		{
			failure
				.lock()
				.unwrap_or_else(PoisonError::into_inner)
				.get_or_insert(WireError {
					status: 500,
					message: "the stream producer failed".to_owned(),
					code: None,
					retry_after: None,
				});
		}
		completion.await;
	}

	/// Sends every item of `items`, each once the client has room for it,
	/// and then the stream's end: done when `items` runs out, failed at the
	/// first item `encode` refuses.
	#[doc(hidden)]
	pub async fn drive<S, I>(self, items: S, encode: fn(I) -> Encoded)
	where
		S: Stream<Item = I> + Send,
	{
		let mut items = std::pin::pin!(items);

		loop {
			if self
				.failure
				.lock()
				.unwrap_or_else(PoisonError::into_inner)
				.is_some()
			{
				return;
			}
			// Wait for room before reading the next item, so nothing is read
			// ahead of what the client asked for.
			match self.credits.acquire().await {
				Ok(permit) => permit.forget(),
				Err(_) => return,
			}

			let Some(item) = items.next().await else {
				self.end(EndReason::Done).await;

				return;
			};

			match encode(item) {
				Ok((value, cursor)) => {
					let frame = ServerFrame::Item {
						stream: self.stream,
						value,
						cursor,
					};

					if !self.send(&frame).await {
						return;
					}
				}
				Err(error) => {
					self.fail(&error);
					return;
				}
			}
		}
	}

	/// Ends the stream as failed, as when its body refused to start.
	#[doc(hidden)]
	pub fn fail(&self, error: &Error) {
		self.failure
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.get_or_insert_with(|| {
				WireError {
					status: error.status(),
					message: error.message().to_owned(),
					code: error.code().map(str::to_owned),
					retry_after: error.retry_after().map(crate::operation::whole_seconds),
				}
			});
	}

	async fn end(&self, reason: EndReason) {
		self.send(&reason.frame(self.stream)).await;
	}

	async fn send(&self, frame: &ServerFrame) -> bool {
		let Ok(text) = serde_json::to_string(frame) else {
			return false;
		};

		if text.len() > MAX_FRAME_BYTES {
			let error = WireError {
				status: 413,
				message: format!("a stream frame may contain at most {MAX_FRAME_BYTES} bytes"),
				code: None,
				retry_after: None,
			};

			if let Ok(text) = serde_json::to_string(&EndReason::Failed(error).frame(self.stream)) {
				let _ = self.enqueue(text, true).await;
			}

			return false;
		}

		self.enqueue(text, matches!(frame, ServerFrame::End { .. }))
			.await
	}

	async fn enqueue(&self, text: String, terminal: bool) -> bool {
		let sent = self
			.out
			.send(Frame {
				stream: self.stream,
				generation: Arc::clone(&self.generation),
				text,
				terminal,
			})
			.await
			.is_ok();

		if sent && terminal {
			self.terminal.store(true, Ordering::Release);
		}

		sent
	}
}

/// The most credits a stream may hold at once, however many the client
/// grants.
const MAX_CREDITS: usize = 1 << 20;

struct Running {
	generation: Arc<()>,
	task: tokio::task::JoinHandle<()>,
	credits: Arc<Semaphore>,
}

// A cancelled or unwinding session must not detach a stream producer.
impl Drop for Running {
	fn drop(&mut self) {
		self.task.abort();
	}
}

/// The streams one connection holds open, for whichever session serves
/// it.
#[cfg_attr(
	not(feature = "live-streams"),
	allow(dead_code, reason = "only a live session opens streams")
)]
pub(crate) struct StreamSet {
	running: HashMap<u32, Running>,
	/// What running streams send, forwarded in order.
	frames: mpsc::Receiver<Frame>,
	out: mpsc::Sender<Frame>,
}

#[cfg_attr(
	not(feature = "live-streams"),
	allow(dead_code, reason = "only a live session opens streams")
)]
impl StreamSet {
	/// No streams yet, with room for `capacity` frames the session hasn't
	/// taken.
	pub(crate) fn new(capacity: usize) -> Self {
		let (out, frames) = mpsc::channel(capacity);

		Self {
			running: HashMap::new(),
			frames,
			out,
		}
	}

	/// Opens the stream `name` that `service`'s live socket serves, with
	/// room for `credits` items, replacing any stream the client already
	/// numbered `stream`. Returns the frame that says why it didn't open.
	pub(crate) fn open(
		&mut self,
		service: Option<&str>,
		stream: u32,
		name: &str,
		args: Value,
		credits: u32,
		cx: StreamOpen,
	) -> Option<ServerFrame> {
		self.running.remove(&stream);

		let Some(registration) = registration_in(service, name) else {
			let error = WireError::invalid(format!("no stream is named `{name}`"));

			return Some(EndReason::Failed(error).frame(stream));
		};

		if self.running.len() >= MAX_STREAMS {
			let error =
				WireError::invalid(format!("a connection may hold {MAX_STREAMS} streams open"));

			return Some(EndReason::Failed(error).frame(stream));
		}

		let credits = Arc::new(Semaphore::new(
			usize::try_from(credits)
				.unwrap_or(MAX_CREDITS)
				.min(MAX_CREDITS),
		));
		let sink = StreamSink::new(stream, self.out.clone(), Arc::clone(&credits));
		let generation = Arc::clone(&sink.generation);
		let task = tokio::spawn(sink.supervise(registration.open, args, cx));
		self.running.insert(
			stream,
			Running {
				generation,
				task,
				credits,
			},
		);

		None
	}

	/// Gives a stream room for `n` more items.
	pub(crate) fn credit(&self, stream: u32, n: u32) {
		let Some(running) = self.running.get(&stream) else {
			return;
		};
		let room = MAX_CREDITS.saturating_sub(running.credits.available_permits());
		let n = usize::try_from(n).unwrap_or(room).min(room);

		if n > 0 {
			running.credits.add_permits(n);
		}
	}

	/// Stops a stream the client no longer wants.
	pub(crate) fn cancel(&mut self, stream: u32) {
		self.running.remove(&stream);
	}

	/// The next frame a running stream produced.
	pub(crate) async fn recv(&mut self) -> Option<Frame> {
		self.frames.recv().await
	}

	/// The text to send for a frame a stream produced, unless the stream
	/// was cancelled or replaced since. A stream's last frame closes it.
	pub(crate) fn accept(&mut self, frame: Frame) -> Option<String> {
		let running = self.running.get(&frame.stream)?;

		if !Arc::ptr_eq(&running.generation, &frame.generation) {
			return None;
		}

		if frame.terminal {
			self.running.remove(&frame.stream);
		}

		Some(frame.text)
	}

	/// Stops every stream still running. Returns what to send: the frames
	/// they had already produced, then an interrupted end for each that
	/// hadn't finished.
	pub(crate) async fn interrupt(&mut self) -> Vec<String> {
		for running in self.running.values() {
			running.task.abort();
		}

		for running in self.running.values_mut() {
			let _ = (&mut running.task).await;
		}

		let mut texts = Vec::new();

		while let Ok(frame) = self.frames.try_recv() {
			texts.extend(self.accept(frame));
		}

		for (stream, _) in self.running.drain() {
			texts.extend(serde_json::to_string(&EndReason::Interrupted.frame(stream)).ok());
		}

		texts
	}

	/// Stops every stream, saying nothing: the connection is gone.
	pub(crate) fn abort_all(&mut self) {
		self.running.clear();
	}
}

/// What a session's own tests look at.
#[cfg(test)]
#[cfg_attr(
	not(feature = "live"),
	allow(dead_code, reason = "the live session's tests read these")
)]
impl StreamSet {
	pub(crate) fn is_empty(&self) -> bool {
		self.running.is_empty()
	}

	pub(crate) fn contains(&self, stream: u32) -> bool {
		self.running.contains_key(&stream)
	}

	/// Whether any producer is still running.
	pub(crate) fn producing(&self) -> bool {
		self.running
			.values()
			.any(|running| !running.task.is_finished())
	}

	/// How many frames the session hasn't taken yet.
	pub(crate) fn queued(&self) -> usize {
		self.frames.len()
	}

	/// Holds `task` as the producer of `stream`, and returns the generation
	/// its frames carry.
	pub(crate) fn hold(&mut self, stream: u32, task: tokio::task::JoinHandle<()>) -> Arc<()> {
		let generation = Arc::new(());
		self.running.insert(
			stream,
			Running {
				generation: Arc::clone(&generation),
				task,
				credits: Arc::new(Semaphore::new(1)),
			},
		);

		generation
	}
}

fn encode<T: Serialize>(item: &T) -> Result<Value> {
	serde_json::to_value(item).map_err(|_| Error::server("a stream's item isn't JSON"))
}

/// An item that can't fail.
#[doc(hidden)]
pub fn plain<T: Serialize>(item: T) -> Encoded {
	Ok((encode(&item)?, None))
}

/// An item that may be the error the stream fails with.
#[doc(hidden)]
pub fn fallible<T: Serialize>(item: Result<T>) -> Encoded {
	plain(item?)
}

/// A resumable stream's item, with its cursor.
#[doc(hidden)]
#[expect(
	clippy::needless_pass_by_value,
	reason = "every encoder takes the item the stream yields, by value"
)]
pub fn resumable<T: Serialize + Resumable>(item: T) -> Encoded {
	let cursor = serde_json::to_value(item.cursor())
		.map_err(|_| Error::server("a stream's cursor isn't JSON"))?;
	Ok((encode(&item)?, Some(cursor)))
}

/// A resumable stream's item that may be the error the stream fails with.
#[doc(hidden)]
pub fn resumable_fallible<T: Serialize + Resumable>(item: Result<T>) -> Encoded {
	resumable(item?)
}
