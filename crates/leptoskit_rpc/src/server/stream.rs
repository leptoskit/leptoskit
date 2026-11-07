//! The streams a server knows, and how one runs: its items go out only as
//! fast as the client grants credits, so the server never reads ahead of
//! what the client has room for.

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

use super::QueryCx;
use crate::Error;
use crate::Result;
use crate::StreamOperation;
use crate::live::EndReason;
use crate::live::MAX_FRAME_BYTES;
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
pub type Open = fn(Value, QueryCx, StreamSink) -> BoxFuture<'static, ()>;

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
	cx: QueryCx,
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

/// Where a running stream sends its items: the connection's session, gated
/// by the credits its client granted.
#[derive(Debug)]
#[cfg_attr(
	not(feature = "live"),
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
		not(feature = "live"),
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
		not(feature = "live"),
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
		not(feature = "live"),
		allow(dead_code, reason = "only live sessions supervise producers")
	)]
	pub(crate) async fn supervise(self, open: Open, args: Value, cx: QueryCx) {
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
