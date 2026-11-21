//! What carries a live connection: the WebSocket at the live path and its
//! server-sent events fallback, as thin adapters around whichever server
//! owns the connection's session.
//!
//! A [`Host`] is that server: the full [`Live`](super::Live), which serves
//! queries, mutations, channels and streams on a database, or
//! [`LiveStreams`](super::LiveStreams), which serves streams alone.

pub(crate) mod socket;
pub(crate) mod sse;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::time::Duration;

use axum::http::HeaderMap;
use axum::http::header::ORIGIN;
use tokio::sync::mpsc;

use super::Identify;
use super::Identity;
use crate::live::ServerFrame;
use crate::live::WireError;

/// How many frames may queue for a slow connection. An unfinished
/// transition retains its remaining chunks; later changes coalesce into
/// one current snapshot when transport capacity returns.
pub(crate) const OUTGOING: usize = 32;

/// How many client frames may wait for a busy session before the transport
/// stops reading.
pub(crate) const INCOMING: usize = 64;

/// How many frames one connection may send before the server serves it no
/// further. Most frames run against the database (a query, a mutation, a
/// session lookup), so a client that cycles subscriptions at line rate is
/// closed instead of fed.
///
/// The budget is a token bucket: [`FRAMES_PER_SECOND`] tokens return every
/// second, and a connection starts with a burst of [`FRAME_BURST`] so a
/// reconnecting client can re-subscribe to everything it had in one go.
pub(crate) const FRAME_BURST: u64 = 512;
pub(crate) const FRAMES_PER_SECOND: u64 = 256;

/// How long a connection's client may leave a frame unread before the
/// connection closes: the heartbeat's interval.
pub(crate) const STALL: Duration = Duration::from_secs(crate::live::HEARTBEAT_SECONDS);

/// What a session writes for its transport to send.
#[derive(Clone, Debug)]
pub(crate) enum Outbound {
	/// A server frame as JSON, with the version a transition brings the
	/// client to, which server-sent events carry as the event's `id`.
	Frame { text: String, version: Option<u64> },
	/// A keep-alive: a WebSocket ping, or an event stream comment.
	Heartbeat,
}

impl Outbound {
	pub(crate) fn frame(frame: &ServerFrame, version: Option<u64>) -> Option<Self> {
		let text = serde_json::to_string(frame).ok()?;
		Some(Self::Frame { text, version })
	}
}

/// What a transport hands its session.
#[derive(Debug)]
pub(crate) enum Incoming {
	/// A client frame.
	Frame(crate::live::ClientFrame),
	/// Something that isn't one, answered with a `done` for request 0.
	Invalid(WireError),
}

/// Who may post frames to a connection: the caller its identity names,
/// which changes when the connection authenticates.
pub(crate) type CallerSlot = Arc<Mutex<Option<String>>>;

/// A connection a transport has opened, for [`Host::serve`].
pub(crate) struct Opened {
	/// The connection's number, from [`Host::next_number`].
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
	#[cfg_attr(
		not(feature = "live"),
		allow(dead_code, reason = "only live queries have versions to resume from")
	)]
	pub resume_floor: Option<u64>,
	/// Kept current with the identity's caller.
	pub caller: CallerSlot,
	/// Where the connection came from, for the server's guards.
	pub address: Option<std::net::IpAddr>,
}

/// A server that owns live sessions, as its transports see it.
pub(crate) trait Host: Clone + Send + Sync + 'static {
	/// How a request becomes a caller.
	fn identifier(&self) -> &dyn Identify;

	/// What runs before every operation a connection sends.
	fn guards(&self) -> &crate::server::Guards;

	/// The one origin a browser connection may come from, when one is set.
	fn allowed_origin(&self) -> Option<&str>;

	/// How long a frame may wait to be sent before the connection closes.
	fn stall(&self) -> Duration;

	/// A fresh connection number.
	fn next_number(&self) -> u32;

	/// The server-sent event connections that take posted frames.
	fn event_streams(&self) -> MutexGuard<'_, HashMap<String, sse::Handle>>;

	/// Runs a connection until its client goes, its transport closes, or
	/// the server drains it.
	fn serve(&self, opened: Opened) -> impl Future<Output = ()> + Send;

	/// Whether a request's `Origin` may connect. A native app sends none
	/// and authenticates with a token.
	fn origin_ok(&self, headers: &HeaderMap) -> bool {
		let Some(allowed) = self.allowed_origin() else {
			return true;
		};

		match headers.get(ORIGIN) {
			None => true,
			Some(origin) => {
				origin
					.to_str()
					.is_ok_and(|origin| origin.trim_end_matches('/') == allowed)
			}
		}
	}
}

/// The answer to a caller the server can't serve live: a service, which
/// reads the app's data by calling its operations, never by watching them.
pub(crate) fn refuse_service(identity: &Identity) -> Option<crate::Error> {
	crate::server::is_service(identity).then(|| {
		crate::Error::forbidden(
			"a service can't hold a live connection to the app: call its operations instead",
		)
	})
}
