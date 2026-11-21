//! Checks that run before every operation a client sends: how often a
//! caller may ask, how long a call may take, how large its arguments may
//! be, and any check of the app's own.
//!
//! ```ignore
//! let rpc = rpc::server::Server::new(db)
//! 	.identify(auth.clone())
//! 	.client_ip_header("fly-client-ip")
//! 	.rate_limit(rpc::server::RateLimit::new());
//! ```
//!
//! An operation narrows its own limits where it declares them:
//!
//! ```ignore
//! #[rpc::mutation(rate_limit = 5 / 1m, timeout = 10s, max_body = 64KiB)]
//! async fn send_invite(cx: &rpc::MutationCx, email: String) -> rpc::Result<()> { … }
//! ```
//!
//! Guards run on the calls a client sends: `POST {path}/<name>`, and the
//! subscriptions, mutations, actions and streams a live connection sends.
//! Calls the server makes itself, such as `client::call` while a page
//! renders, are already inside a request that was checked, and aren't
//! checked again.

use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;
use std::time::Instant;

use leptoskit_query::BoxFuture;

use super::Identity;
use crate::Error;
use crate::Kind;

/// At most `max` calls in any `per`, refilled evenly: a caller who has
/// waited `per` may spend all `max` at once, and after that gets one more
/// every `per / max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rate {
	/// Calls allowed at once, and in any `per`.
	pub max: u32,
	/// How long `max` calls take to refill.
	pub per: Duration,
}

impl Rate {
	/// At most `max` calls in any `per`.
	#[must_use]
	pub const fn new(max: u32, per: Duration) -> Self {
		Self { max, per }
	}

	/// At most `max` calls a second.
	#[must_use]
	pub const fn per_second(max: u32) -> Self {
		Self::new(max, Duration::from_secs(1))
	}

	/// At most `max` calls a minute.
	#[must_use]
	pub const fn per_minute(max: u32) -> Self {
		Self::new(max, Duration::from_mins(1))
	}

	/// At most `max` calls an hour.
	#[must_use]
	pub const fn per_hour(max: u32) -> Self {
		Self::new(max, Duration::from_hours(1))
	}

	/// How long one call takes to refill.
	fn interval(self) -> Duration {
		if self.max == 0 {
			return self.per;
		}

		self.per / self.max
	}
}

impl fmt::Display for Rate {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{} per {:?}", self.max, self.per)
	}
}

/// What one operation allows beyond the server's own limits:
/// `#[rpc::query(rate_limit = 10 / 1m, timeout = 5s, max_body = 64KiB)]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OperationLimits {
	/// How often one caller may call this operation, counted apart from
	/// the server's [`RateLimit`]: a call has to fit both.
	pub rate: Option<Rate>,
	/// How long a call may run before it is answered `504` (`timed_out`)
	/// instead. A mutation that runs out of time rolls back.
	pub timeout: Option<Duration>,
	/// The largest arguments, in bytes, a call over HTTP may send: in place
	/// of the server's [`max_body`](super::Server::max_body).
	pub max_body: Option<usize>,
}

impl OperationLimits {
	/// No limits of the operation's own.
	pub const NONE: Self = Self {
		rate: None,
		timeout: None,
		max_body: None,
	};

	/// With `rate`.
	#[must_use]
	pub const fn with_rate(mut self, rate: Rate) -> Self {
		self.rate = Some(rate);
		self
	}

	/// With `timeout`.
	#[must_use]
	pub const fn with_timeout(mut self, timeout: Duration) -> Self {
		self.timeout = Some(timeout);
		self
	}

	/// With `max_body`.
	#[must_use]
	pub const fn with_max_body(mut self, bytes: usize) -> Self {
		self.max_body = Some(bytes);
		self
	}
}

/// How a call arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Via {
	/// One HTTP request: `POST {path}/<name>`.
	Http,
	/// A frame on a live connection: a WebSocket, or server-sent events.
	Live,
}

/// One call a client sent, as a [`Guard`] sees it.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Call<'a> {
	/// The operation's name, as in its route.
	pub name: &'a str,
	/// Its kind: a query, mutation, action, or stream.
	pub kind: Kind,
	/// Who is calling.
	pub identity: &'a Identity,
	/// Where the call came from, as
	/// [`client_ip_header`](super::Server::client_ip_header) says to read
	/// it; `None` when the server can't tell.
	pub address: Option<IpAddr>,
	/// How it arrived.
	pub via: Via,
	/// The operation's own limits.
	pub limits: &'a OperationLimits,
}

impl Call<'_> {
	/// Whether the caller is another part of the app: a service the app
	/// calls, or the app calling a service. Neither is a stranger.
	#[must_use]
	pub fn is_internal(&self) -> bool {
		self.identity.caller.as_deref().is_some_and(|caller| {
			caller == super::APP_CALLER || caller.starts_with(super::SERVICE_CALLER_PREFIX)
		})
	}
}

/// A check that runs before every operation a client sends, over HTTP or a
/// live connection, after the caller is identified, after the server's
/// [`RateLimit`], and before the operation's body runs.
///
/// ```ignore
/// /// Only people who are signed in may call anything but `public.*`.
/// struct SignedIn;
///
/// impl rpc::server::Guard for SignedIn {
/// 	fn check<'a>(&'a self, call: &'a rpc::server::Call<'a>) -> BoxFuture<'a, rpc::Result<()>> {
/// 		let allowed = call.name.starts_with("public.") || call.identity.caller.is_some();
/// 		Box::pin(std::future::ready(if allowed {
/// 			Ok(())
/// 		} else {
/// 			Err(rpc::Error::unauthorized("sign in first"))
/// 		}))
/// 	}
/// }
///
/// let rpc = Server::new(db).identify(auth).guard(SignedIn);
/// ```
pub trait Guard: Send + Sync + 'static {
	/// Admits `call`, or refuses it with the error its caller receives.
	///
	/// # Errors
	///
	/// Whatever refuses the call: usually [`Error::rate_limited`],
	/// [`Error::unauthorized`], or [`Error::forbidden`]. Over a live
	/// connection it answers that one frame and the connection stays open.
	fn check<'a>(&'a self, call: &'a Call<'a>) -> BoxFuture<'a, Result<(), Error>>;
}

impl<G: Guard> Guard for Arc<G> {
	fn check<'a>(&'a self, call: &'a Call<'a>) -> BoxFuture<'a, Result<(), Error>> {
		(**self).check(call)
	}
}

/// Where a call came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum ClientIp {
	/// The connection's own address.
	#[default]
	Peer,
	/// The last value of this header, which a proxy in front of the server
	/// sets, or the connection's own when it is missing.
	Header(http::HeaderName),
}

/// The guards a server runs, and how it reads where a call came from.
/// Shared by the HTTP routes and the live connections.
#[derive(Clone, Default)]
pub(crate) struct Guards {
	/// Counts each operation's own `rate_limit`, and, once the server sets
	/// one, every caller's allowance.
	rate: Arc<RateLimit>,
	/// Whether the server set a [`RateLimit`] of its own, so callers are
	/// counted on every call and not only on operations with a
	/// `rate_limit`.
	limits_callers: bool,
	/// The app's own, in the order they were added.
	custom: Vec<Arc<dyn Guard>>,
	client_ip: ClientIp,
	/// The default for [`OperationLimits::timeout`].
	#[cfg_attr(
		not(any(feature = "live", feature = "postgres", feature = "sqlite")),
		allow(dead_code, reason = "only calls and live operations have a timeout")
	)]
	timeout: Option<Duration>,
}

impl Guards {
	pub(crate) fn push(&mut self, guard: Arc<dyn Guard>) {
		self.custom.push(guard);
	}

	pub(crate) fn set_rate_limit(&mut self, limit: RateLimit) {
		self.rate = Arc::new(limit);
		self.limits_callers = true;
	}

	pub(crate) fn set_client_ip(&mut self, client_ip: ClientIp) {
		self.client_ip = client_ip;
	}

	#[cfg(any(feature = "live", feature = "postgres", feature = "sqlite"))]
	pub(crate) fn set_timeout(&mut self, timeout: Option<Duration>) {
		self.timeout = timeout;
	}

	/// Where `parts` came from.
	pub(crate) fn address(&self, parts: &http::request::Parts) -> Option<IpAddr> {
		let from_header = match &self.client_ip {
			ClientIp::Peer => None,
			ClientIp::Header(name) => {
				parts
					.headers
					.get_all(name)
					.iter()
					.next_back()
					.and_then(|value| value.to_str().ok())
					.and_then(one_address)
			}
		};

		from_header.or_else(|| {
			parts
				.extensions
				.get::<axum::extract::ConnectInfo<SocketAddr>>()
				.map(|axum::extract::ConnectInfo(peer)| peer.ip())
		})
	}

	/// Counts `call` against the caller's allowance and its operation's,
	/// then runs every guard on it, in the order they were added, until one
	/// refuses.
	pub(crate) async fn check(&self, call: &Call<'_>) -> Result<(), Error> {
		if let Some((caller, rate)) = self.rate.caller(call) {
			if self.limits_callers {
				self.rate.spend(&caller, rate).await?;
			}

			if let Some(rate) = call.limits.rate {
				self.rate
					.spend(&format!("{}\0{caller}", call.name), rate)
					.await?;
			}
		}

		for guard in &self.custom {
			guard.check(call).await?;
		}

		Ok(())
	}

	#[cfg(any(feature = "live", feature = "postgres", feature = "sqlite"))]
	/// Runs `future` within `limits`' timeout, or the server's.
	pub(crate) async fn within<T>(
		&self,
		limits: &OperationLimits,
		future: impl Future<Output = Result<T, Error>>,
	) -> Result<T, Error> {
		let Some(timeout) = limits.timeout.or(self.timeout) else {
			return future.await;
		};

		tokio::time::timeout(timeout, future)
			.await
			.unwrap_or_else(|_| Err(timed_out(timeout)))
	}
}

#[cfg(any(feature = "live", feature = "postgres", feature = "sqlite"))]
/// What a call that ran out of time answers.
pub(crate) fn timed_out(timeout: Duration) -> Error {
	Error::Refused {
		code: TIMED_OUT.to_owned(),
		message: format!("the call took longer than its {timeout:?} and was stopped"),
		status: 504,
	}
}

/// The code a client reads for a call that ran out of time.
pub const TIMED_OUT: &str = "timed_out";

/// The code a client reads for arguments over the size limit.
pub const TOO_LARGE: &str = "too_large";

#[cfg(any(feature = "postgres", feature = "sqlite"))]
/// What a call whose arguments are over `limit` bytes answers.
pub(crate) fn too_large(limit: usize) -> Error {
	Error::Refused {
		code: TOO_LARGE.to_owned(),
		message: format!("send at most {limit} bytes of arguments"),
		status: 413,
	}
}

impl fmt::Debug for Guards {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("Guards")
			.field("rate", &self.rate)
			.field("limits_callers", &self.limits_callers)
			.field("custom", &self.custom.len())
			.field("client_ip", &self.client_ip)
			.field("timeout", &self.timeout)
			.finish()
	}
}

/// One address, as a proxy writes it: alone, or with a port.
fn one_address(text: &str) -> Option<IpAddr> {
	let text = text.trim();
	text.parse()
		.ok()
		.or_else(|| text.parse::<SocketAddr>().ok().map(|peer| peer.ip()))
}

/// Where a [`RateLimit`] keeps its counts.
///
/// [`MemoryStore`] is the default: fast, but each server process counts on
/// its own, so behind a load balancer with three servers a caller gets
/// three times the rate. Implement this over Redis, or the database, to
/// share one count between them.
pub trait RateStore: Send + Sync + 'static {
	/// Spends one call from `key`'s allowance under `rate`, or answers how
	/// long until one is free.
	///
	/// # Errors
	///
	/// How long the caller should wait.
	fn hit<'a>(&'a self, key: &'a str, rate: Rate) -> BoxFuture<'a, Result<(), Duration>>;
}

/// The most callers [`MemoryStore`] counts apart. Past it, a caller not yet
/// counted shares one allowance with every other such caller, [`OVERFLOW`]
/// times one caller's, so the counts can't be made to grow without end.
const MAX_KEYS: usize = 100_000;

/// How many callers' worth of calls the callers that found no room share.
const OVERFLOW: u32 = 100;

/// What the callers that found no room are counted under.
const OVERFLOW_KEY: &str = "\0overflow";

/// How many counts there are before full ones are forgotten.
const SWEEP_AT: usize = 10_000;

/// The least time between two sweeps: a sweep reads every count, so a full
/// map isn't swept on every call.
const SWEEP_EVERY: Duration = Duration::from_secs(10);

/// Counts in this process's memory: one allowance per key, refilled evenly,
/// with a bounded number of keys. See [`RateStore`] for what it doesn't
/// share.
#[derive(Debug, Default)]
pub struct MemoryStore {
	buckets: Mutex<Buckets>,
}

#[derive(Debug, Default)]
struct Buckets {
	/// When each key's allowance is whole again (GCRA's "theoretical
	/// arrival time"), with the rate it was counted under.
	full_at: HashMap<String, (Instant, Rate)>,
	swept: Option<Instant>,
}

impl MemoryStore {
	/// An empty store.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Spends one call from `key` at `now`.
	pub(crate) fn spend(&self, key: &str, rate: Rate, now: Instant) -> Result<(), Duration> {
		let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);

		if rate.max == 0 {
			return Err(rate.per);
		}

		if buckets.full_at.len() >= SWEEP_AT
			&& buckets
				.swept
				.is_none_or(|swept| now.duration_since(swept) >= SWEEP_EVERY)
		{
			buckets.full_at.retain(|_, (full_at, _)| *full_at > now);
			buckets.swept = Some(now);
		}

		if !buckets.full_at.contains_key(key) && buckets.full_at.len() >= MAX_KEYS {
			let shared = Rate {
				max: rate.max.saturating_mul(OVERFLOW),
				per: rate.per,
			};
			return buckets.spend(OVERFLOW_KEY, shared, now);
		}

		buckets.spend(key, rate, now)
	}
}

impl Buckets {
	/// The generic cell rate algorithm: a call is let through when the
	/// allowance would not be more than `per` in debt after it.
	fn spend(&mut self, key: &str, rate: Rate, now: Instant) -> Result<(), Duration> {
		let interval = rate.interval();
		let full_at = self
			.full_at
			.get(key)
			.map_or(now, |(full_at, _)| (*full_at).max(now));
		let next = full_at + interval;
		let allowed_from = next.checked_sub(rate.per).unwrap_or(now);

		if allowed_from > now {
			return Err(allowed_from - now);
		}

		self.full_at.insert(key.to_owned(), (next, rate));
		Ok(())
	}
}

impl RateStore for MemoryStore {
	fn hit<'a>(&'a self, key: &'a str, rate: Rate) -> BoxFuture<'a, Result<(), Duration>> {
		Box::pin(std::future::ready(self.spend(key, rate, Instant::now())))
	}
}

impl<S: RateStore> RateStore for Arc<S> {
	fn hit<'a>(&'a self, key: &'a str, rate: Rate) -> BoxFuture<'a, Result<(), Duration>> {
		(**self).hit(key, rate)
	}
}

/// Who a [`RateLimit`] counts a caller as.
pub type CallerKey = Arc<dyn Fn(&Identity) -> Option<String> + Send + Sync>;

/// How often each caller may call the server's operations, counted before
/// every call a client sends:
/// [`Server::rate_limit`](super::Server::rate_limit).
///
/// A caller nobody signed in is counted by their address, with IPv6
/// addresses counted by their /64, which one machine is usually given. A
/// signed-in caller is counted by who they are, so people behind one
/// office network don't share an allowance. The app's own services, and the
/// app calling a service, aren't counted unless
/// [`internal`](Self::internal) says to.
///
/// An operation's own `rate_limit` is counted apart, per caller, on top,
/// with the same keys and store. It is counted whether or not the server
/// sets a `RateLimit`: without one, by these defaults.
///
/// A refused call is answered `429` with `Retry-After`, which clients read
/// as [`FnError::Refused`](crate::FnError::Refused) with the code
/// `rate_limited` and how long to wait.
///
/// ```ignore
/// let rpc = Server::new(db)
/// 	.identify(auth)
/// 	.rate_limit(
/// 		RateLimit::new()
/// 			.anonymous(Rate::per_minute(60))
/// 			.signed_in(Rate::per_minute(600)),
/// 	);
/// ```
#[derive(Clone)]
pub struct RateLimit {
	anonymous: Rate,
	signed_in: Rate,
	internal: Option<Rate>,
	store: Arc<dyn RateStore>,
	key: CallerKey,
}

impl Default for RateLimit {
	fn default() -> Self {
		Self::new()
	}
}

impl RateLimit {
	/// 120 calls a minute from an address nobody signed in from, 600 a
	/// minute for someone signed in, internal callers not counted, kept
	/// in a [`MemoryStore`].
	///
	/// A page that renders on the server calls in process and isn't
	/// counted; a page that loads in the browser and a client-side
	/// navigation each make a handful of calls, so neither allowance is felt
	/// by people using the app, while a script polling it settles at two or
	/// ten calls a second.
	#[must_use]
	pub fn new() -> Self {
		Self {
			anonymous: Rate::per_minute(120),
			signed_in: Rate::per_minute(600),
			internal: None,
			store: Arc::new(MemoryStore::new()),
			key: Arc::new(|identity: &Identity| identity.caller.clone()),
		}
	}

	/// How often one address nobody is signed in from may call.
	#[must_use]
	pub fn anonymous(mut self, rate: Rate) -> Self {
		self.anonymous = rate;
		self
	}

	/// How often one signed-in caller may call.
	#[must_use]
	pub fn signed_in(mut self, rate: Rate) -> Self {
		self.signed_in = rate;
		self
	}

	/// How often one internal caller (a service, or the app calling a
	/// service) may call; `None`, the default, doesn't count them.
	#[must_use]
	pub fn internal(mut self, rate: Option<Rate>) -> Self {
		self.internal = rate;
		self
	}

	/// Where the counts are kept: a [`MemoryStore`] by default.
	#[must_use]
	pub fn store(mut self, store: impl RateStore) -> Self {
		self.store = Arc::new(store);
		self
	}

	/// Who a signed-in caller is counted as. By default their
	/// [`Identity::caller`], which for `leptoskit_auth` is one session, so
	/// each device someone signs in on has its own allowance. Return the
	/// person's account to count every session of theirs together, or
	/// `None` to count the caller by address.
	///
	/// ```ignore
	/// RateLimit::new().key(|identity| identity.context.get::<Account>().map(|account| account.id.to_string()))
	/// ```
	#[must_use]
	pub fn key(
		mut self,
		key: impl Fn(&Identity) -> Option<String> + Send + Sync + 'static,
	) -> Self {
		self.key = Arc::new(key);
		self
	}

	/// Who `call` is counted as, and under which rate; `None` when it isn't
	/// counted.
	fn caller(&self, call: &Call<'_>) -> Option<(String, Rate)> {
		if call.is_internal() {
			let rate = self.internal?;
			return Some((
				format!(
					"internal:{}",
					call.identity.caller.as_deref().unwrap_or_default()
				),
				rate,
			));
		}

		match (self.key)(call.identity) {
			Some(key) => Some((format!("caller:{key}"), self.signed_in)),
			None => Some((address_key(call.address), self.anonymous)),
		}
	}

	/// Spends one call from `key` under `rate`.
	async fn spend(&self, key: &str, rate: Rate) -> Result<(), Error> {
		self.store.hit(key, rate).await.map_err(Error::rate_limited)
	}
}

impl fmt::Debug for RateLimit {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("RateLimit")
			.field("anonymous", &self.anonymous)
			.field("signed_in", &self.signed_in)
			.field("internal", &self.internal)
			.finish_non_exhaustive()
	}
}

/// The key calls from `address` are counted under: the address, or for
/// IPv6 its /64, since one network hands a machine all of that.
fn address_key(address: Option<IpAddr>) -> String {
	match address.map(|address| address.to_canonical()) {
		Some(IpAddr::V4(address)) => format!("address:{address}"),
		Some(IpAddr::V6(address)) => {
			let [a, b, c, d, ..] = address.segments();
			format!("address:{a:x}:{b:x}:{c:x}:{d:x}::/64")
		}
		None => "address:unknown".to_owned(),
	}
}

#[cfg(test)]
mod tests {
	use std::net::Ipv4Addr;
	use std::net::Ipv6Addr;

	use super::*;

	#[test]
	fn a_rate_lets_a_burst_through_then_refills_evenly() {
		let store = MemoryStore::new();
		let rate = Rate::new(3, Duration::from_secs(3));
		let start = Instant::now();

		for _ in 0..3 {
			assert_eq!(store.spend("a", rate, start), Ok(()));
		}

		// The burst is spent: the next call waits for one to refill.
		assert_eq!(store.spend("a", rate, start), Err(Duration::from_secs(1)));
		let later = start + Duration::from_millis(400);
		assert_eq!(
			store.spend("a", rate, later),
			Err(Duration::from_millis(600))
		);
		// One refills every second, not the whole burst at once.
		let one = start + Duration::from_secs(1);
		assert_eq!(store.spend("a", rate, one), Ok(()));
		assert!(store.spend("a", rate, one).is_err());
		// Someone else's allowance is their own.
		assert_eq!(store.spend("b", rate, one), Ok(()));
		// After a full `per`, the whole burst is back.
		let rested = one + Duration::from_secs(3);
		for _ in 0..3 {
			assert_eq!(store.spend("a", rate, rested), Ok(()));
		}
		assert!(store.spend("a", rate, rested).is_err());
		// A rate of nothing lets nothing through.
		assert_eq!(
			store.spend("c", Rate::new(0, Duration::from_secs(9)), start),
			Err(Duration::from_secs(9))
		);
	}

	#[test]
	fn a_flood_of_keys_shares_one_allowance_past_the_limit() {
		let store = MemoryStore::new();
		let rate = Rate::per_minute(1);
		let start = Instant::now();

		for key in 0..MAX_KEYS {
			assert_eq!(store.spend(&key.to_string(), rate, start), Ok(()));
		}

		// A key already counted is counted as before.
		assert!(store.spend("0", rate, start).is_err());

		// New keys share one allowance a hundred times as large.
		for late in 0..OVERFLOW as usize {
			assert_eq!(store.spend(&format!("late-{late}"), rate, start), Ok(()));
		}
		assert!(store.spend("late-next", rate, start).is_err());

		// Once allowances are whole again they are forgotten, and there is
		// room again.
		let next = start + Duration::from_mins(2);
		assert_eq!(store.spend("late-next", rate, next), Ok(()));
		assert_eq!(store.buckets.lock().expect("the buckets").full_at.len(), 1);
	}

	#[test]
	fn addresses_are_counted_by_machine() {
		assert_eq!(
			address_key(Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)))),
			"address:203.0.113.7"
		);
		// An IPv4 address written as IPv6 is the IPv4 address.
		assert_eq!(
			address_key(Some(IpAddr::V6(
				Ipv4Addr::new(203, 0, 113, 7).to_ipv6_mapped()
			))),
			"address:203.0.113.7"
		);
		// Every address in one /64 is one machine.
		let one: Ipv6Addr = "2001:db8:1:2:aaaa::1".parse().expect("an address");
		let other: Ipv6Addr = "2001:db8:1:2:bbbb::9".parse().expect("an address");
		assert_eq!(
			address_key(Some(IpAddr::V6(one))),
			address_key(Some(IpAddr::V6(other)))
		);
		assert_eq!(
			address_key(Some(IpAddr::V6(one))),
			"address:2001:db8:1:2::/64"
		);
		assert_eq!(address_key(None), "address:unknown");
	}

	fn call<'a>(identity: &'a Identity, limits: &'a OperationLimits) -> Call<'a> {
		Call {
			name: "notes",
			kind: Kind::Query,
			identity,
			address: Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1))),
			via: Via::Http,
			limits,
		}
	}

	fn limited(limit: RateLimit) -> Guards {
		let mut guards = Guards::default();
		guards.set_rate_limit(limit);
		guards
	}

	#[tokio::test]
	async fn callers_are_counted_by_who_they_are() {
		let limit = limited(
			RateLimit::new()
				.anonymous(Rate::per_minute(1))
				.signed_in(Rate::per_minute(2)),
		);
		let none = OperationLimits::NONE;

		let anonymous = Identity::anonymous();
		limit
			.check(&call(&anonymous, &none))
			.await
			.expect("the first");
		let refused = limit
			.check(&call(&anonymous, &none))
			.await
			.expect_err("over the anonymous rate");
		assert_eq!(refused.status(), 429);
		assert_eq!(refused.code(), Some(crate::RATE_LIMITED));
		assert!(refused.retry_after().is_some());

		// Someone signed in from the same address has their own allowance.
		let person = Identity::anonymous().with_caller("session-1");
		limit.check(&call(&person, &none)).await.expect("theirs");
		limit.check(&call(&person, &none)).await.expect("theirs");
		assert!(limit.check(&call(&person, &none)).await.is_err());

		// A service isn't counted by default.
		let service = Identity::anonymous().with_caller("service:analytics");
		for _ in 0..10 {
			limit
				.check(&call(&service, &none))
				.await
				.expect("not counted");
		}

		let counted = limited(RateLimit::new().internal(Some(Rate::per_minute(1))));
		counted
			.check(&call(&service, &none))
			.await
			.expect("the first");
		assert!(counted.check(&call(&service, &none)).await.is_err());
	}

	#[tokio::test]
	async fn an_operation_counts_its_own_rate_on_top() {
		// Without a server-wide limit too.
		let limit = Guards::default();
		let tight = OperationLimits::NONE.with_rate(Rate::per_minute(1));
		let person = Identity::anonymous().with_caller("session-1");

		limit
			.check(&call(&person, &tight))
			.await
			.expect("the first");
		assert!(limit.check(&call(&person, &tight)).await.is_err());
		// Another operation is not held up by it.
		let mut other = call(&person, &OperationLimits::NONE);
		other.name = "other";
		limit.check(&other).await.expect("another operation");
		limit.check(&other).await.expect("no server-wide limit");
	}

	#[tokio::test]
	async fn a_key_can_count_every_session_of_one_person_together() {
		let limit = limited(
			RateLimit::new()
				.signed_in(Rate::per_minute(1))
				.key(|identity| {
					identity
						.caller
						.as_deref()
						.map(|caller| caller.split('/').next().unwrap_or(caller).to_owned())
				}),
		);
		let none = OperationLimits::NONE;
		let phone = Identity::anonymous().with_caller("ada/phone");
		let laptop = Identity::anonymous().with_caller("ada/laptop");

		limit.check(&call(&phone, &none)).await.expect("the first");
		assert!(limit.check(&call(&laptop, &none)).await.is_err());
	}

	#[cfg(any(feature = "live", feature = "postgres", feature = "sqlite"))]
	#[tokio::test]
	async fn guards_run_in_order_and_time_calls_out() {
		struct Refuse;

		impl Guard for Refuse {
			fn check<'a>(&'a self, _call: &'a Call<'a>) -> BoxFuture<'a, Result<(), Error>> {
				Box::pin(std::future::ready(Err(Error::forbidden("no"))))
			}
		}

		let mut guards = Guards::default();
		let anonymous = Identity::anonymous();
		let none = OperationLimits::NONE;
		guards.check(&call(&anonymous, &none)).await.expect("none");
		guards.push(Arc::new(Refuse));
		assert_eq!(
			guards.check(&call(&anonymous, &none)).await,
			Err(Error::forbidden("no"))
		);

		let quick = OperationLimits::NONE.with_timeout(Duration::from_millis(10));
		let slow = async {
			tokio::time::sleep(Duration::from_secs(5)).await;
			Ok(())
		};
		let error = guards.within(&quick, slow).await.expect_err("timed out");
		assert_eq!(error.status(), 504);
		assert_eq!(error.code(), Some(TIMED_OUT));

		// The server's timeout holds when the operation has none.
		guards.set_timeout(Some(Duration::from_millis(10)));
		let slow = async {
			tokio::time::sleep(Duration::from_secs(5)).await;
			Ok(())
		};
		assert!(guards.within(&none, slow).await.is_err());
		assert_eq!(guards.within(&none, async { Ok(1) }).await, Ok(1));
	}

	#[test]
	fn the_address_comes_from_the_proxy_header_when_told() {
		let mut request = http::Request::new(());
		request
			.headers_mut()
			.append("fly-client-ip", "203.0.113.1".parse().expect("a value"));
		request.headers_mut().append(
			"fly-client-ip",
			"203.0.113.9:4000".parse().expect("a value"),
		);
		request
			.extensions_mut()
			.insert(axum::extract::ConnectInfo(SocketAddr::from((
				[10, 0, 0, 2],
				9000,
			))));
		let (parts, ()) = request.into_parts();

		let mut guards = Guards::default();
		assert_eq!(guards.address(&parts), Some(IpAddr::from([10, 0, 0, 2])));

		guards.set_client_ip(ClientIp::Header(http::HeaderName::from_static(
			"fly-client-ip",
		)));
		// The last header is the proxy's own.
		assert_eq!(guards.address(&parts), Some(IpAddr::from([203, 0, 113, 9])));

		guards.set_client_ip(ClientIp::Header(http::HeaderName::from_static("x-real-ip")));
		assert_eq!(guards.address(&parts), Some(IpAddr::from([10, 0, 0, 2])));
	}
}
