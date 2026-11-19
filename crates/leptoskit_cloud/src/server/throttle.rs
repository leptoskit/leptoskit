//! How often one caller may ask.
//!
//! Every operation is counted against who asked, once they are known: a
//! person by their account, whichever way they signed in; an API token by
//! its own ID, and with its organisation's other tokens; a worker by its
//! own ID; and everyone else by the address the request came from. An
//! operation sent over a live connection is counted as one sent alone is.
//! A caller past their [`Rate`] is answered `429` (`rate_limited`) until
//! the window ends. Sign-in and the OAuth provider
//! count their own attempts, in the database; these counters only keep one
//! caller from taking the control plane's time from the rest.
//!
//! The counters are fixed windows kept in this process's memory. A control
//! plane run as several processes allows each caller its rate in each of
//! them, and a restart forgets what was counted.
//!
//! Callers who are known and addresses are counted apart, so a flood of
//! addresses never costs a person, a token or a worker their turn.

use std::collections::HashMap;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use leptoskit_axum::axum::extract::ConnectInfo;
use leptoskit_rpc as rpc;

/// The most addresses counted at once. Past it, an address not yet counted
/// shares one count with every other such address, [`OVERFLOW`] times one
/// address's: the counters can't be made to grow without end, and callers
/// who are known are counted elsewhere.
const MAX_ADDRESSES: usize = 100_000;

/// How many addresses' worth of requests the addresses that found no room
/// share.
const OVERFLOW: u32 = 100;

/// What the addresses that found no room are counted under.
const OVERFLOW_KEY: &str = "address:others";

/// How many counters there are before the ones whose windows ended are
/// forgotten.
const SWEEP_AT: usize = 10_000;

/// The least time between two passes that forget ended windows: a pass
/// reads every counter, so a full map isn't swept on every request.
const SWEEP_EVERY: Duration = Duration::from_secs(10);

/// At most `max` requests in each `window`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
	pub max: u32,
	pub window: Duration,
}

impl Rate {
	/// At most `max` requests a minute.
	#[must_use]
	pub const fn a_minute(max: u32) -> Self {
		Self {
			max,
			window: Duration::from_mins(1),
		}
	}
}

/// How often each kind of caller may ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimits {
	/// A person, over the console and the CLI together.
	pub person: Rate,
	/// One API token.
	pub token: Rate,
	/// All of one organisation's API tokens together: an organisation may
	/// make many tokens, and they don't multiply what it may ask.
	pub organization_tokens: Rate,
	/// One worker: claims, renewals, logs and reports.
	pub worker: Rate,
	/// One address, for requests that are nobody's: no credential, or one
	/// that isn't anyone's.
	pub anonymous: Rate,
	/// One organisation's checkout and portal pages, each of which is a
	/// call to the payment provider.
	pub payment_pages: Rate,
}

impl Default for RateLimits {
	/// 600 a minute for a person and for a token, 3000 for an
	/// organisation's tokens together, 6000 for a worker, 120 for an
	/// address nobody is signed in from, and 10 payment pages in ten
	/// minutes for an organisation.
	fn default() -> Self {
		Self {
			person: Rate::a_minute(600),
			token: Rate::a_minute(600),
			organization_tokens: Rate::a_minute(3000),
			worker: Rate::a_minute(6000),
			anonymous: Rate::a_minute(120),
			payment_pages: Rate {
				max: 10,
				window: Duration::from_mins(10),
			},
		}
	}
}

#[derive(Debug, Clone, Copy)]
struct Window {
	started: Instant,
	length: Duration,
	count: u32,
}

impl Window {
	fn over(&self, now: Instant) -> bool {
		now.duration_since(self.started) >= self.length
	}
}

/// One set of counters.
#[derive(Debug, Default)]
struct Counters {
	windows: HashMap<String, Window>,
	swept: Option<Instant>,
}

impl Counters {
	/// Counts one request under `key`, or answers `None` when `key` isn't
	/// counted yet and `room` says there is no room for it.
	fn count(
		&mut self,
		key: &str,
		rate: Rate,
		now: Instant,
		room: Option<usize>,
	) -> Option<Result<(), Duration>> {
		if let Some(window) = self.windows.get_mut(key)
			&& !window.over(now)
		{
			if window.count >= rate.max {
				return Some(Err(window
					.length
					.saturating_sub(now.duration_since(window.started))));
			}

			window.count += 1;
			return Some(Ok(()));
		}

		if rate.max == 0 {
			return Some(Err(rate.window));
		}

		if self.windows.len() >= SWEEP_AT
			&& self
				.swept
				.is_none_or(|swept| now.duration_since(swept) >= SWEEP_EVERY)
		{
			self.windows.retain(|_, window| !window.over(now));
			self.swept = Some(now);
		}

		if room.is_some_and(|room| self.windows.len() >= room && !self.windows.contains_key(key)) {
			return None;
		}

		self.windows.insert(
			key.to_owned(),
			Window {
				started: now,
				length: rate.window,
				count: 1,
			},
		);
		Some(Ok(()))
	}
}

/// The counters: the callers who are known, and the addresses of the rest.
#[derive(Debug, Default)]
pub(crate) struct Throttle {
	known: Mutex<Counters>,
	addresses: Mutex<Counters>,
}

fn refusal(wait: Duration) -> rpc::Error {
	rpc::Error::too_many(format!(
		"rate_limited: too many requests; try again in {} seconds",
		wait.as_secs().max(1)
	))
}

impl Throttle {
	fn count(&self, key: &str, rate: Rate, now: Instant) -> Result<(), Duration> {
		self.known
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.count(key, rate, now, None)
			.unwrap_or(Ok(()))
	}

	fn count_address(
		&self,
		address: Option<IpAddr>,
		rate: Rate,
		now: Instant,
	) -> Result<(), Duration> {
		let mut addresses = self
			.addresses
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);

		if let Some(counted) =
			addresses.count(&address_key(address), rate, now, Some(MAX_ADDRESSES))
		{
			return counted;
		}

		let shared = Rate {
			max: rate.max.saturating_mul(OVERFLOW),
			window: rate.window,
		};
		addresses
			.count(OVERFLOW_KEY, shared, now, None)
			.unwrap_or(Ok(()))
	}

	/// Counts one request under `key`, which names a caller who is known:
	/// someone or something with a row of its own, so there are only as
	/// many keys as there are of those.
	///
	/// # Errors
	///
	/// `TooMany` (`rate_limited`) when `rate` is already used up.
	pub(crate) fn hit(&self, key: &str, rate: Rate) -> rpc::Result<()> {
		self.count(key, rate, Instant::now()).map_err(refusal)
	}

	/// Counts one request from `address`, for a caller who is nobody.
	///
	/// # Errors
	///
	/// `TooMany` (`rate_limited`) when `rate` is already used up.
	pub(crate) fn hit_address(&self, address: Option<IpAddr>, rate: Rate) -> rpc::Result<()> {
		self.count_address(address, rate, Instant::now())
			.map_err(refusal)
	}
}

/// The key requests from `address` are counted under: the address, or for
/// IPv6 its /64, since one network hands a machine all of that.
fn address_key(address: Option<IpAddr>) -> String {
	match address {
		Some(IpAddr::V4(address)) => format!("address:{address}"),
		Some(IpAddr::V6(address)) => {
			if let Some(address) = address.to_ipv4_mapped() {
				format!("address:{address}")
			} else {
				let [a, b, c, d, ..] = address.segments();
				format!("address:{a:x}:{b:x}:{c:x}:{d:x}::/64")
			}
		}
		None => "address:unknown".to_owned(),
	}
}

/// One address, as a proxy writes it: alone, or with a port.
fn one_address(text: &str) -> Option<IpAddr> {
	let text = text.trim();
	text.parse()
		.ok()
		.or_else(|| text.parse::<SocketAddr>().ok().map(|peer| peer.ip()))
}

/// Where `request` came from: the address in `header` when the control
/// plane is told a proxy in front of it sets one, and otherwise the
/// connection's own. The header's whole value must be one address, and the
/// last such header is read: a list is a client's word followed by a
/// proxy's, and a header a client sent comes before the one a proxy adds.
/// A request without the header, or with one that isn't an address, is
/// counted by its connection: never by nothing.
pub(crate) fn address(request: &http::request::Parts, header: Option<&str>) -> Option<IpAddr> {
	header
		.and_then(|name| request.headers.get_all(name).iter().next_back())
		.and_then(|value| value.to_str().ok())
		.and_then(one_address)
		.or_else(|| {
			request
				.extensions
				.get::<ConnectInfo<SocketAddr>>()
				.map(|ConnectInfo(peer)| peer.ip())
		})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_caller_is_counted_in_windows_of_their_own() {
		let throttle = Throttle::default();
		let rate = Rate {
			max: 2,
			window: Duration::from_secs(60),
		};
		let start = Instant::now();
		assert_eq!(throttle.count("person:a", rate, start), Ok(()));
		assert_eq!(throttle.count("person:a", rate, start), Ok(()));
		let later = start + Duration::from_secs(45);
		assert_eq!(
			throttle.count("person:a", rate, later),
			Err(Duration::from_secs(15))
		);
		// Someone else's count is their own.
		assert_eq!(throttle.count("person:b", rate, later), Ok(()));
		// A new window starts when the old one ends, not before.
		let next = start + Duration::from_secs(60);
		assert_eq!(throttle.count("person:a", rate, next), Ok(()));
		assert_eq!(throttle.count("person:a", rate, next), Ok(()));
		assert!(throttle.count("person:a", rate, next).is_err());
		// A rate of nothing lets nothing through.
		let none = Rate { max: 0, ..rate };
		assert_eq!(
			throttle.count("person:c", none, start),
			Err(Duration::from_secs(60))
		);
	}

	fn numbered(number: usize) -> Option<IpAddr> {
		u32::try_from(number)
			.ok()
			.map(|number| IpAddr::V4(std::net::Ipv4Addr::from(number)))
	}

	#[test]
	fn a_flood_of_addresses_costs_nobody_known_their_turn() {
		let throttle = Throttle::default();
		let rate = Rate::a_minute(1);
		let start = Instant::now();

		for address in 0..MAX_ADDRESSES {
			assert_eq!(
				throttle.count_address(numbered(address), rate, start),
				Ok(())
			);
		}

		// Full: an address already counted is counted as before, and the
		// ones that found no room share a count a hundred times as large.
		assert!(throttle.count_address(numbered(0), rate, start).is_err());

		for late in 0..OVERFLOW as usize {
			assert_eq!(
				throttle.count_address(numbered(MAX_ADDRESSES + late), rate, start),
				Ok(())
			);
		}

		assert!(
			throttle
				.count_address(numbered(MAX_ADDRESSES + 500), rate, start)
				.is_err()
		);
		assert_eq!(
			throttle
				.addresses
				.lock()
				.expect("the counters")
				.windows
				.len(),
			MAX_ADDRESSES + 1
		);
		// Someone known is counted elsewhere, and has their turn.
		assert_eq!(throttle.count("person:a", rate, start), Ok(()));
		// Once the windows end they are forgotten, and there is room again.
		let next = start + Duration::from_secs(60);
		assert_eq!(
			throttle.count_address(numbered(MAX_ADDRESSES + 500), rate, next),
			Ok(())
		);
		assert_eq!(
			throttle
				.addresses
				.lock()
				.expect("the counters")
				.windows
				.len(),
			1
		);
	}

	#[test]
	fn ended_windows_are_forgotten_now_and_then() {
		let throttle = Throttle::default();
		let rate = Rate::a_minute(1);
		let start = Instant::now();

		for caller in 0..SWEEP_AT {
			assert_eq!(throttle.count(&caller.to_string(), rate, start), Ok(()));
		}

		let size = || throttle.known.lock().expect("the counters").windows.len();
		// Nothing has ended, so the pass forgets nothing, and nobody known
		// is ever refused for room.
		assert_eq!(throttle.count("one more", rate, start), Ok(()));
		assert_eq!(size(), SWEEP_AT + 1);
		// A pass was just made, so the next waits its turn.
		let ended = start + Duration::from_secs(70);
		assert_eq!(
			throttle.count("another", rate, start + SWEEP_EVERY / 2),
			Ok(())
		);
		assert_eq!(size(), SWEEP_AT + 2);
		assert_eq!(throttle.count("a third", rate, ended), Ok(()));
		assert_eq!(size(), 1);
	}

	#[test]
	fn an_ipv6_address_is_counted_by_its_network() {
		let key = |text: &str| address_key(Some(text.parse().expect("an address")));
		assert_eq!(key("203.0.113.7"), "address:203.0.113.7");
		assert_eq!(key("::ffff:203.0.113.7"), "address:203.0.113.7");
		assert_eq!(
			key("2001:db8:1:2:aaaa:bbbb:cccc:dddd"),
			key("2001:db8:1:2::1")
		);
		assert_ne!(key("2001:db8:1:2::1"), key("2001:db8:1:3::1"));
		assert_eq!(address_key(None), "address:unknown");
	}

	#[test]
	fn an_address_is_the_proxys_word_only_when_told_so() {
		let request = |forwarded: Option<&str>, peer: Option<&str>| {
			let mut builder = http::Request::builder();

			if let Some(forwarded) = forwarded {
				builder = builder.header("x-real-ip", forwarded);
			}

			let (mut parts, ()) = builder.body(()).expect("a request").into_parts();

			if let Some(peer) = peer {
				let peer: SocketAddr = peer.parse().expect("an address");
				parts.extensions.insert(ConnectInfo(peer));
			}

			parts
		};
		let direct = request(Some("198.51.100.1"), Some("203.0.113.7:4000"));
		// Not told of a proxy, the header is anyone's to write.
		assert_eq!(address(&direct, None), "203.0.113.7".parse().ok());
		assert_eq!(
			address(&direct, Some("x-real-ip")),
			"198.51.100.1".parse().ok()
		);
		assert_eq!(
			address(&request(Some("[2001:db8::1]:443"), None), Some("x-real-ip")),
			"2001:db8::1".parse().ok()
		);
		// A list is partly the client's word, so it is nobody's: the
		// connection's address is counted, as it is with no header or one
		// that isn't an address.
		for wrong in [Some("198.51.100.1, 10.0.0.1"), Some("not one"), None] {
			assert_eq!(
				address(&request(wrong, Some("203.0.113.7:4000")), Some("x-real-ip")),
				"203.0.113.7".parse().ok(),
				"{wrong:?}"
			);
		}

		assert_eq!(address(&request(None, None), Some("x-real-ip")), None);
		// A header the client sent comes before the proxy's own.
		let (mut twice, ()) = http::Request::builder()
			.header("x-real-ip", "198.51.100.1")
			.header("x-real-ip", "192.0.2.9")
			.body(())
			.expect("a request")
			.into_parts();
		twice.extensions.insert(ConnectInfo(
			"203.0.113.7:4000"
				.parse::<SocketAddr>()
				.expect("an address"),
		));
		assert_eq!(address(&twice, Some("x-real-ip")), "192.0.2.9".parse().ok());
	}
}
