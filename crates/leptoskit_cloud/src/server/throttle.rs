//! How often one caller may ask.
//!
//! Every operation is counted against who asked, once they are known: a
//! person by their account, whichever way they signed in; an API token and
//! a worker each by their own ID; and everyone else by the address the
//! request came from. A caller past their [`Rate`] is answered `429`
//! (`rate_limited`) until the window ends. Sign-in and the OAuth provider
//! count their own attempts, in the database; these counters only keep one
//! caller from taking the control plane's time from the rest.
//!
//! The counters are fixed windows kept in this process's memory. A control
//! plane run as several processes allows each caller its rate in each of
//! them, and a restart forgets what was counted.

use std::collections::HashMap;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use leptoskit_axum::axum::extract::ConnectInfo;
use leptoskit_rpc as rpc;

/// The most callers counted at once. Past it, a caller not yet counted is
/// refused until a window ends: the counters can't be made to grow without
/// end.
const MAX_COUNTED: usize = 200_000;

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
	/// 600 a minute for a person and for a token, 6000 for a worker, 120
	/// for an address nobody is signed in from, and 10 payment pages in
	/// ten minutes for an organisation.
	fn default() -> Self {
		Self {
			person: Rate::a_minute(600),
			token: Rate::a_minute(600),
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

/// The counters.
#[derive(Debug, Default)]
pub(crate) struct Throttle {
	windows: Mutex<HashMap<String, Window>>,
}

impl Throttle {
	/// Counts one request under `key`.
	///
	/// # Errors
	///
	/// How long until the window ends, when `rate` is already used up.
	fn count(&self, key: &str, rate: Rate, now: Instant) -> Result<(), Duration> {
		let mut windows = self
			.windows
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);

		if let Some(window) = windows.get_mut(key)
			&& !window.over(now)
		{
			if window.count >= rate.max {
				return Err(window
					.length
					.saturating_sub(now.duration_since(window.started)));
			}

			window.count += 1;
			return Ok(());
		}

		if windows.len() >= MAX_COUNTED {
			windows.retain(|_, window| !window.over(now));
		}

		if rate.max == 0 || windows.len() >= MAX_COUNTED {
			return Err(rate.window);
		}

		windows.insert(
			key.to_owned(),
			Window {
				started: now,
				length: rate.window,
				count: 1,
			},
		);
		Ok(())
	}

	/// Counts one request under `key`.
	///
	/// # Errors
	///
	/// `TooMany` (`rate_limited`) when `rate` is already used up.
	pub(crate) fn hit(&self, key: &str, rate: Rate) -> rpc::Result<()> {
		self.count(key, rate, Instant::now()).map_err(|wait| {
			rpc::Error::too_many(format!(
				"rate_limited: too many requests; try again in {} seconds",
				wait.as_secs().max(1)
			))
		})
	}
}

/// The key requests from `address` are counted under: the address, or for
/// IPv6 its /64, since one network hands a machine all of that.
pub(crate) fn address_key(address: Option<IpAddr>) -> String {
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

/// Where `request` came from: the first address in `header` when the
/// control plane is told a proxy in front of it sets one, and otherwise
/// the connection's own.
pub(crate) fn address(request: &http::request::Parts, header: Option<&str>) -> Option<IpAddr> {
	match header {
		Some(name) => {
			request
				.headers
				.get(name)
				.and_then(|value| value.to_str().ok())
				.and_then(|value| value.split(',').next())
				.and_then(|value| value.trim().parse().ok())
		}
		None => {
			request
				.extensions
				.get::<ConnectInfo<SocketAddr>>()
				.map(|ConnectInfo(peer)| peer.ip())
		}
	}
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

	#[test]
	fn the_counters_cannot_grow_without_end() {
		let throttle = Throttle::default();
		let rate = Rate::a_minute(1);
		let start = Instant::now();

		for caller in 0..MAX_COUNTED {
			assert_eq!(throttle.count(&caller.to_string(), rate, start), Ok(()));
		}

		// Full: a caller not yet counted waits, and one already counted is
		// counted as before.
		assert!(throttle.count("one more", rate, start).is_err());
		assert!(throttle.count("0", rate, start).is_err());
		// Once the windows end they are forgotten, and there is room again.
		let next = start + Duration::from_secs(60);
		assert_eq!(throttle.count("one more", rate, next), Ok(()));
		assert_eq!(throttle.windows.lock().expect("the counters").len(), 1);
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
		let chained = request(Some("198.51.100.1, 10.0.0.1"), None);
		assert_eq!(
			address(&chained, Some("x-real-ip")),
			"198.51.100.1".parse().ok()
		);
		assert_eq!(address(&request(None, None), Some("x-real-ip")), None);
		assert_eq!(
			address(&request(Some("not one"), None), Some("x-real-ip")),
			None
		);
	}
}
