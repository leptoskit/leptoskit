//! Which hosts a Web Push request may leave for, and which addresses it
//! may connect to when it does.
//!
//! A subscription's endpoint is attacker-shaped material: anyone who can
//! drive an enrollment can hand the service a URL. The policy answers
//! with a fixed allowlist, not a denylist — arbitrary hosts are refused
//! by default, so there is no clever form that gets through. The shipped
//! providers' hosts are exact-matched. An operator may name extra push
//! services — a plain host name, checked at startup and refused by name
//! if it carries anything else — and those get connection-time
//! validation: the host is resolved within a bounded time, every address
//! it answers is checked against the IANA special-purpose registries,
//! and the request is pinned to the addresses that passed, so a rebind
//! between check and connect has nothing to land on.
//!
//! Refusals here never echo the endpoint's URL: an attacker's input
//! returns as a policy answer, not as a quoted string.

use std::time::Duration;

use super::Pinned;

/// The push-service hosts leptoskit pushes to, exact and lower case.
/// Chrome and Android Chrome answer to `fcm.googleapis.com`, and Firefox
/// to Mozilla's autopush. A subscription at any other host — outside
/// [`SHIPPED_DOMAINS`] — is refused at enrollment, so a lookalike
/// (`fcm.googleapis.com.attacker.test`), a port trick, or a credential in
/// the URL never gets as far as a delivery.
pub(crate) const SHIPPED_HOSTS: &[&str] =
	&["fcm.googleapis.com", "updates.push.services.mozilla.com"];

/// The push services that hand out endpoints under many subdomains of one
/// domain, matched on a label boundary: `web.push.apple.com` is under
/// `push.apple.com`, `webpush.apple.com` and `push.apple.com.attacker.test`
/// are not. Safari's subscriptions live on Apple's `push.apple.com`
/// subdomains, which Apple tells senders to allow as `*.push.apple.com`,
/// and Edge on Windows subscribes through the Windows Push Notification
/// Services, whose channel URLs Microsoft says to accept on the
/// `notify.windows.com` domain whatever their subdomain. The domain itself
/// is not an endpoint.
pub(crate) const SHIPPED_DOMAINS: &[&str] = &["push.apple.com", "notify.windows.com"];

/// Whether `host`, lower case, is one of the shipped push services.
fn is_shipped(host: &str) -> bool {
	SHIPPED_HOSTS.contains(&host)
		|| SHIPPED_DOMAINS.iter().any(|domain| {
			host.strip_suffix(domain)
				.and_then(|label| label.strip_suffix('.'))
				.is_some_and(is_host_shape)
		})
}

/// How long a pinned host's resolution may take before the attempt gives
/// up on it as a transient failure.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// The operator's extra push services, and the promise that requests to
/// them are DNS-pinned.
#[derive(Clone, Debug, Default)]
pub(crate) struct HostPolicy {
	extra: Vec<String>,
}

impl HostPolicy {
	/// The policy with operator-approved extra hosts, lower cased and
	/// trimmed. A name that isn't only a host — a scheme, a port, a
	/// credential, a space — is a configuration mistake, so it is refused
	/// by name here rather than silently dropped: an operator should see
	/// the entry that never worked, not discover it at the first push.
	///
	/// # Errors
	///
	/// The malformed entry, named.
	pub(crate) fn with_extra_hosts(hosts: &[String]) -> Result<Self, String> {
		for host in hosts {
			let lowered = host.trim().to_ascii_lowercase();

			if !is_host_shape(&lowered) {
				return Err(format!(
					"Web Push: the approved extra host {host:?} isn't a plain host name"
				));
			}
		}

		Ok(Self {
			extra: hosts
				.iter()
				.map(|host| host.trim().to_ascii_lowercase())
				.collect(),
		})
	}

	/// Whether `url` may be pushed to, and how. Shipped hosts answer
	/// plain; extra hosts answer with the pinning the connection needs.
	///
	/// # Errors
	///
	/// A message naming why the host is refused, without echoing the
	/// endpoint.
	pub(crate) fn check(&self, url: &url::Url) -> Result<Option<Pinned>, String> {
		let Some(host) = url.host_str() else {
			return Err("the endpoint names no host".into());
		};
		let host = host.to_ascii_lowercase();

		if is_shipped(&host) {
			return Ok(None);
		}

		if self.extra.contains(&host) {
			return Ok(Some(Pinned {
				host,
				port: url.port().unwrap_or(443),
				addrs: Vec::new(),
			}));
		}

		Err(
			"the endpoint's push service isn't one this server pushes to: its endpoints must be \
			 one of the supported push services, or a host the operator approved"
				.to_owned(),
		)
	}
}

/// Whether a name is only a host: letters, digits, dots, hyphens, and an
/// optional bracketed IPv6 form, with no scheme, path, port, or
/// credentials hiding in it.
fn is_host_shape(host: &str) -> bool {
	!host.is_empty()
		&& host.len() <= 253
		&& !host.contains('/')
		&& !host.contains(':')
		&& !host.contains('@')
		&& !host.contains(' ')
		&& host.split('.').all(|label| {
			!label.is_empty()
				&& label
					.bytes()
					.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
		})
}

/// Validates every address a host answered and returns them for pinning.
/// One private, reserved, or link-local address in the answer refuses the
/// whole host: a resolver that answers a public address and a loopback
/// one together is answering like an attacker.
///
/// # Errors
///
/// A message naming the refused address's class. The host named here is
/// an operator-approved one, never an endpoint's URL.
pub(crate) fn validate_addresses(
	host: &str,
	addrs: &[std::net::IpAddr],
) -> Result<Vec<std::net::IpAddr>, String> {
	if addrs.is_empty() {
		return Err(format!("{host} resolved to no addresses"));
	}

	for addr in addrs {
		let class = private_class(*addr);
		if let Some(class) = class {
			return Err(format!("{host} resolved to a {class} address"));
		}
	}

	Ok(addrs.to_vec())
}

/// Resolves `host` within a bounded time and validates what it answers,
/// for a pinned request. `host` is an operator-approved extra host, so a
/// resolver failure is transient — the network's, not the policy's — and
/// the delivery that asked retries rather than ending.
///
/// # Errors
///
/// A message from the resolver or the validator.
pub(crate) async fn resolve_pinned(host: &str, port: u16) -> Result<Pinned, String> {
	let resolved = tokio::time::timeout(RESOLVE_TIMEOUT, tokio::net::lookup_host((host, port)))
		.await
		.map_err(|_| format!("{host} didn't resolve in time"))?
		.map_err(|error| format!("{host} didn't resolve: {error}"))?;

	let mut addrs = resolved.map(|socket| socket.ip()).collect::<Vec<_>>();
	addrs.sort();
	addrs.dedup();
	let addrs = validate_addresses(host, &addrs)?;

	Ok(Pinned {
		host: host.to_owned(),
		port,
		addrs,
	})
}

/// The class of a non-global address, for refusals: `None` when the
/// address is globally routable, which is all this policy connects to.
///
/// The table mirrors the IANA IPv4 and IPv6 Special-Purpose Address
/// Registries as the standard library's own unstable
/// `Ipv4Addr::is_global`/`Ipv6Addr::is_global` is written from them —
/// including the ranges a naive check misses (240/4, the IETF protocol
/// assignments, `ORCHID`, `SRv6`, the transition networks) — with two
/// deliberate differences: multicast is refused in both families (a
/// push has no business joining a group), and the transition networks
/// that embed IPv4 (IPv4-mapped, NAT64's well-known prefixes) are judged
/// by their embedded addresses, so a private address stays private
/// however it is wrapped.
fn private_class(addr: std::net::IpAddr) -> Option<&'static str> {
	match addr {
		std::net::IpAddr::V4(addr) => v4_class(addr),
		std::net::IpAddr::V6(addr) => v6_class(addr),
	}
}

fn v4_class(addr: std::net::Ipv4Addr) -> Option<&'static str> {
	let [a, b, c, d] = addr.octets();

	if addr.is_loopback() {
		Some("loopback")
	} else if addr.is_private() {
		Some("private")
	} else if addr.is_link_local() {
		Some("link-local")
	} else if a == 0 {
		// "This network".
		Some("reserved")
	} else if addr.is_multicast() || addr.is_broadcast() {
		Some("reserved")
	} else if addr.is_documentation() {
		Some("documentation")
	} else if a == 198 && (b & 0b1111_1110) == 18 {
		// Benchmarking, 198.18.0.0/15.
		Some("benchmarking")
	} else if a == 100 && (b & 0b1100_0000) == 0b0100_0000 {
		// Shared address space, 100.64.0.0/10.
		Some("shared-address")
	} else if a == 192 && b == 0 && c == 0 && !matches!(d, 9 | 10) {
		// The IETF protocol assignments, 192.0.0.0/24; .9 and .10 are
		// anycasts documented as globally reachable.
		Some("reserved")
	} else if a == 192 && b == 88 && c == 99 {
		// The deprecated 6to4/6a44 relay anycast range. The standard
		// library only refuses its single `.2`; the whole /24 is retired,
		// and refusing it all is the safer read.
		Some("reserved")
	} else if a & 0b1111_0000 == 0b1111_0000 {
		// 240.0.0.0/4, reserved for future use (the standard library's
		// unstable `is_reserved`, written plainly).
		Some("reserved")
	} else {
		None
	}
}

/// Whether `segments` sits in the IETF protocol assignments,
/// `2001::/23`.
fn in_ietf_assignments(segments: &[u16; 8]) -> bool {
	matches!(segments, [0x2001, b, _, _, _, _, _, _] if *b < 0x200)
}

/// Whether `segments` is one of the anycast endpoints the registry
/// documents as globally reachable: Port Control Protocol, Teredo's
/// relays, and DNS-SD registration (`2001:1::1` through `::3`), AMT
/// (`2001:3::/32`), and AS112 (`2001:4:112::/48`).
fn globally_reachable_anycast(segments: &[u16; 8], octets: &[u8; 16]) -> bool {
	(u128::from_be_bytes(*octets) & 0xffff_ffff_ffff_ffff_ffff_ffff_ffff_fffc
		== 0x2001_0001_0000_0000_0000_0000_0000_0000
		&& (octets[14], octets[15]) != (0, 0))
		|| matches!(segments, [0x2001, 3, _, _, _, _, _, _])
		|| matches!(segments, [0x2001, 4, 0x112, _, _, _, _, _])
}

fn v6_class(addr: std::net::Ipv6Addr) -> Option<&'static str> {
	let segments = addr.segments();
	let octets = addr.octets();
	let embedded_v4 = || std::net::Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);

	// An IPv4-mapped address is its embedded IPv4 address for checking.
	if matches!(segments, [0, 0, 0, 0, 0, 0xffff, _, _]) {
		return v4_class(embedded_v4());
	}

	// NAT64's well-known prefixes embed the IPv4 address they translate:
	// `64:ff9b::/96` globally, and `64:ff9b:1::/48` on local networks
	// (whose embedded half isn't even a plain address, so the whole /48
	// is refused).
	if matches!(segments, [0x64, 0xff9b, 1, _, _, _, _, _]) {
		return Some("translation");
	}

	if matches!(segments, [0x64, 0xff9b, 0, 0, 0, 0, _, _]) {
		return v4_class(embedded_v4());
	}

	if addr.is_unspecified() {
		Some("unspecified")
	} else if addr.is_loopback() {
		Some("loopback")
	} else if addr.is_multicast() {
		Some("reserved")
	} else if matches!(segments, [0x100, 0, 0, 0, _, _, _, _]) {
		// The discard-only prefix.
		Some("discard")
	} else if matches!(segments, [0x100, 0, 0, 1, _, _, _, _]) {
		// The dummy prefix for benchmarking.
		Some("reserved")
	} else if matches!(segments, [0x2001, 0xdb8, _, _, _, _, _, _]) {
		// Documentation, 2001:db8::/32.
		Some("documentation")
	} else if in_ietf_assignments(&segments) && !globally_reachable_anycast(&segments, &octets) {
		// The IETF protocol assignments under 2001::/23, minus their
		// globally reachable pieces: Teredo (2001::/32), `ORCHID` and
		// `ORCHIDv2` (2001:10::/28, 2001:20::/28), the Drone Remote ID
		// prefix (2001:30::/28), and the registry's future additions.
		Some("reserved")
	} else if matches!(segments, [0x2002, _, _, _, _, _, _, _]) {
		// 6to4. The registry marks global reachability N/A, and the
		// standard library refuses the whole prefix rather than judging
		// the embedded IPv4 address; so does this.
		Some("transition")
	} else if matches!(segments, [0x5f00, ..]) {
		// Segment Routing SIDs.
		Some("reserved")
	} else if addr.is_unique_local() {
		Some("unique-local")
	} else if (segments[0] & 0xffc0) == 0xfe80 {
		Some("link-local")
	} else {
		None
	}
}

#[cfg(test)]
mod tests {
	use super::HostPolicy;
	use super::SHIPPED_HOSTS;
	use super::private_class;
	use super::validate_addresses;

	/// The shipped hosts pass plain: no pinning, no resolver call.
	#[test]
	fn shipped_hosts_pass_plain() {
		let policy = HostPolicy::with_extra_hosts(&[]).unwrap();

		for host in SHIPPED_HOSTS {
			let url = url::Url::parse(&format!("https://{host}/push/abc")).unwrap();
			assert!(
				policy.check(&url).unwrap().is_none(),
				"{host} needs no pinning"
			);
		}
	}

	/// The lookalike forms an attacker would try: a suffix, a subdomain,
	/// credentials, a scheme swap. A wrong port passes the host policy on
	/// purpose — the port is enrollment's check — and is tested there.
	#[test]
	fn lookalike_hosts_are_refused_without_echoing_the_url() {
		let policy = HostPolicy::with_extra_hosts(&[]).unwrap();
		let refused = [
			"https://fcm.googleapis.com.attacker.test/push",
			"https://attacker.test/fcm.googleapis.com",
			"https://fcm.googleapis.com.evil.test:443/x",
			"https://updates.push.services.mozilla.com.attacker.test/",
			"https://webpush.apple.com@attacker.test/",
			"https://attacker.test/push/1",
		];

		for endpoint in refused {
			let url = url::Url::parse(endpoint).unwrap();
			let refusal = policy.check(&url).unwrap_err();
			assert!(!refusal.contains(endpoint), "no URL echoed: {refusal}");
			assert!(
				!refusal.contains("attacker.test") && !refusal.contains("evil.test"),
				"no host echoed either: {refusal}"
			);
		}

		let port = url::Url::parse("https://fcm.googleapis.com:8443/").unwrap();
		assert!(
			policy.check(&port).is_ok(),
			"the shipped host check is on the host only; the port check is enrollment's"
		);
	}

	/// Endpoints as each major browser hands them out pass plain: Chrome's
	/// FCM, Firefox's autopush, Safari's Apple Web Push on a
	/// `push.apple.com` subdomain, and Edge's Windows Push Notification
	/// Services channel on a `notify.windows.com` subdomain. The domains
	/// match on a label boundary only: a lookalike that ends with the same
	/// letters, the bare domain, a trailing dot, or the domain under an
	/// attacker's own is refused.
	#[test]
	fn real_browser_endpoints_pass_and_lookalikes_never_do() {
		let policy = HostPolicy::with_extra_hosts(&[]).unwrap();

		for endpoint in [
			"https://fcm.googleapis.com/fcm/send/dQw4w9WgXcQ:APA91bFakeChromeEndpoint",
			"https://updates.push.services.mozilla.com/wpush/v2/gAAAAABfakeFirefoxEndpoint",
			"https://web.push.apple.com/QFakeSafariEndpointToken",
			"https://wns2-par02p.notify.windows.com/w/?token=BQYAAADfakeEdgeEndpoint",
			"https://WNS2-BL2P.NOTIFY.WINDOWS.COM/w/?token=BQYAAADfakeEdgeEndpoint",
		] {
			let url = url::Url::parse(endpoint).unwrap();
			assert_eq!(policy.check(&url), Ok(None), "{endpoint} passes plain");
		}

		for endpoint in [
			"https://webpush.apple.com/push/1",
			"https://push.apple.com/push/1",
			"https://evilpush.apple.com/push/1",
			"https://web.push.apple.com.attacker.test/push/1",
			"https://web.push.apple.com./push/1",
			"https://web.push.apple.com@attacker.test/push/1",
			"https://notify.windows.com/w/?token=1",
			"https://attackernotify.windows.com/w/?token=1",
			"https://wns2-par02p.notify.windows.com.attacker.test/w/?token=1",
			"https://notify.windows.com.attacker.test/w/?token=1",
			"https://a_b.push.apple.com/push/1",
		] {
			let url = url::Url::parse(endpoint).unwrap();
			let refusal = policy.check(&url).expect_err(endpoint);
			assert!(!refusal.contains("attacker"), "no host echoed: {refusal}");
		}
	}

	/// An operator-approved host passes with pinning; an unapproved one
	/// still doesn't; a malformed approval is refused by name.
	#[test]
	fn extra_hosts_need_approval_and_a_plain_shape() {
		let policy = HostPolicy::with_extra_hosts(&["push.example.net".to_owned()]).unwrap();
		let approved = url::Url::parse("https://push.example.net/send/1").unwrap();
		let pinned = policy.check(&approved).unwrap();
		assert!(pinned.is_some(), "extra hosts pin");
		assert_eq!(pinned.unwrap().host, "push.example.net");

		let unapproved = url::Url::parse("https://push.example.org/send/1").unwrap();
		assert!(policy.check(&unapproved).is_err());

		for malformed in [
			"https://push.example.net",
			"push.example.net:443",
			"push.example.net/path",
			"user@push.example.net",
			"push exam ple.net",
			"",
		] {
			let error = HostPolicy::with_extra_hosts(&[malformed.to_owned()]).unwrap_err();
			assert!(
				error.contains(malformed),
				"the refusal names the entry: {error}"
			);
		}
	}

	/// Private, reserved, and link-local addresses are named and refused,
	/// IPv4 and IPv6, across the IANA special-purpose registries —
	/// including the ranges a naive check misses (240/4, 192.0.0.0/24,
	/// the IPv4 embedded in transition networks) — and the global ones
	/// pass.
	#[test]
	fn non_global_addresses_have_classes() {
		let refused: &[(&str, std::net::IpAddr)] = &[
			("loopback", "127.0.0.1".parse().unwrap()),
			("loopback", "::1".parse().unwrap()),
			("private", "10.0.0.1".parse().unwrap()),
			("private", "192.168.1.1".parse().unwrap()),
			("private", "172.16.0.1".parse().unwrap()),
			("link-local", "169.254.169.254".parse().unwrap()),
			("reserved", "0.0.0.0".parse().unwrap()),
			("reserved", "0.1.2.3".parse().unwrap()),
			("reserved", "255.255.255.255".parse().unwrap()),
			("reserved", "224.0.0.1".parse().unwrap()),
			("reserved", "239.1.1.1".parse().unwrap()),
			("documentation", "192.0.2.1".parse().unwrap()),
			("documentation", "198.51.100.7".parse().unwrap()),
			("documentation", "203.0.113.9".parse().unwrap()),
			("reserved", "192.0.0.8".parse().unwrap()),
			("reserved", "192.88.99.1".parse().unwrap()),
			("reserved", "240.0.0.1".parse().unwrap()),
			("reserved", "250.1.2.3".parse().unwrap()),
			("shared-address", "100.64.0.1".parse().unwrap()),
			("benchmarking", "198.18.0.1".parse().unwrap()),
			("benchmarking", "198.19.255.255".parse().unwrap()),
			("unique-local", "fc00::1".parse().unwrap()),
			("unique-local", "fd00::1".parse().unwrap()),
			("link-local", "fe80::1".parse().unwrap()),
			("documentation", "2001:db8::1".parse().unwrap()),
			("reserved", "2001:10::1".parse().unwrap()),
			("reserved", "2001:20::1".parse().unwrap()),
			("reserved", "2001:30::1".parse().unwrap()),
			(
				"reserved",
				"2001:0:4136:e378:8000:63bf:3fff:fdd2".parse().unwrap(),
			),
			("reserved", "2001:2::ac15:70f3".parse().unwrap()),
			("reserved", "2001:0::1:1".parse().unwrap()),
			("discard", "100::1".parse().unwrap()),
			("reserved", "100:0:0:1::1".parse().unwrap()),
			("reserved", "5f00::1".parse().unwrap()),
			("reserved", "ff02::1".parse().unwrap()),
			("translation", "64:ff9b:1::a00:1".parse().unwrap()),
			// The transition networks' embedded IPv4 addresses, mapped
			// and otherwise: a private address stays private wrapped.
			("loopback", "::ffff:127.0.0.1".parse().unwrap()),
			("link-local", "::ffff:169.254.169.254".parse().unwrap()),
			("reserved", "::ffff:240.0.0.1".parse().unwrap()),
			("private", "::ffff:10.0.0.1".parse().unwrap()),
			("private", "64:ff9b::a00:1".parse().unwrap()),
			("transition", "2002:a00:1::1".parse().unwrap()),
			("transition", "2002:808:808::1".parse().unwrap()),
			("unspecified", "::".parse().unwrap()),
		];

		for (class, addr) in refused {
			assert_eq!(
				private_class(*addr).map(str::to_owned),
				Some((*class).to_owned()),
				"{addr}"
			);
		}

		let global: &[std::net::IpAddr] = &[
			"1.1.1.1".parse().unwrap(),
			"8.8.8.8".parse().unwrap(),
			"9.255.255.255".parse().unwrap(),
			// The two IETF protocol-assignment anycasts documented as
			// globally reachable.
			"192.0.0.9".parse().unwrap(),
			"192.0.0.10".parse().unwrap(),
			"2606:4700:4700::1111".parse().unwrap(),
			// The globally reachable anycasts the 2001::/23 block
			// documents: Port Control Protocol, AMT, AS112.
			"2001:1::1".parse().unwrap(),
			"2001:1::3".parse().unwrap(),
			"2001:3::1".parse().unwrap(),
			"2001:4:112::1".parse().unwrap(),
			// A NAT64 wrapper around a global IPv4 address is global, the
			// same as its embedded address.
			"64:ff9b::808:808".parse().unwrap(),
		];
		for addr in global {
			assert_eq!(private_class(*addr), None, "{addr}");
		}
	}

	/// One bad address in a resolution refuses the whole answer.
	#[test]
	fn one_private_address_refuses_the_answer() {
		let addrs = [
			"1.1.1.1".parse::<std::net::IpAddr>().unwrap(),
			"127.0.0.1".parse().unwrap(),
		];
		assert!(validate_addresses("mixed.test", &addrs).is_err());

		let addrs = ["1.1.1.1".parse::<std::net::IpAddr>().unwrap()];
		assert!(validate_addresses("public.test", &addrs).is_ok());
		assert!(validate_addresses("empty.test", &[]).is_err());
	}
}
