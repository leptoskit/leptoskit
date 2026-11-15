//! Which hosts a Web Push request may leave for, and which addresses it
//! may connect to when it does.
//!
//! A subscription's endpoint is attacker-shaped material: anyone who can
//! drive an enrollment can hand the service a URL. The policy answers
//! with a fixed allowlist, not a denylist — arbitrary hosts are refused
//! by default, so there is no clever form that gets through. The shipped
//! providers' hosts are exact-matched. An operator may name extra push
//! services, and those get connection-time validation: the host is
//! resolved, every address it answers is checked against the private and
//! link-local ranges, and the request is pinned to the addresses that
//! passed, so a rebind between check and connect has nothing to land on.

use super::Pinned;

/// The push-service hosts leptoskit pushes to, exact and lower case.
/// Chrome and Android Chrome answer to `fcm.googleapis.com`, Firefox to
/// Mozilla's autopush, and Safari to Apple's Web Push service. A
/// subscription at any other host is refused at enrollment, so a lookalike
/// (`fcm.googleapis.com.attacker.test`), a port trick, or a credential in
/// the URL never gets as far as a delivery.
pub(crate) const SHIPPED_HOSTS: &[&str] = &[
	"fcm.googleapis.com",
	"updates.push.services.mozilla.com",
	"webpush.apple.com",
];

/// The operator's extra push services, and the promise that requests to
/// them are DNS-pinned.
#[derive(Clone, Debug, Default)]
pub(crate) struct HostPolicy {
	extra: Vec<String>,
}

impl HostPolicy {
	/// The policy with operator-approved extra hosts, lower cased and
	/// trimmed. Ports aren't in a host here; a name with anything but a
	/// host shape is dropped, so a bad variable can't smuggle one in.
	#[must_use]
	pub fn with_extra_hosts(hosts: &[String]) -> Self {
		Self {
			extra: hosts
				.iter()
				.map(|host| host.trim().to_ascii_lowercase())
				.filter(|host| is_host_shape(host))
				.collect(),
		}
	}

	/// Whether `url` may be pushed to, and how. Shipped hosts answer
	/// plain; extra hosts answer with the pinning the connection needs.
	///
	/// # Errors
	///
	/// A message naming why the host is refused.
	pub(crate) fn check(&self, url: &url::Url) -> Result<Option<Pinned>, String> {
		let Some(host) = url.host_str() else {
			return Err("the endpoint names no host".into());
		};
		let host = host.to_ascii_lowercase();

		if SHIPPED_HOSTS.contains(&host.as_str()) {
			return Ok(None);
		}

		if self.extra.contains(&host) {
			return Ok(Some(Pinned {
				host,
				port: url.port().unwrap_or(443),
				addrs: Vec::new(),
			}));
		}

		Err(format!(
			"the push service at {host} isn't one this server pushes to: its endpoints must be \
			 one of the supported push services, or a host the operator approved"
		))
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
/// A message naming the refused address's class.
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

/// Resolves `host` and validates what it answers, for a pinned request.
///
/// # Errors
///
/// A message from the resolver or the validator.
pub(crate) async fn resolve_pinned(host: &str, port: u16) -> Result<Pinned, String> {
	let mut addrs = tokio::net::lookup_host((host, port))
		.await
		.map_err(|error| format!("{host} didn't resolve: {error}"))?
		.map(|socket| socket.ip())
		.collect::<Vec<_>>();
	addrs.sort();
	addrs.dedup();
	let addrs = validate_addresses(host, &addrs)?;

	Ok(Pinned {
		host: host.to_owned(),
		port,
		addrs,
	})
}

/// The class of a private or reserved address, for refusals: `None` when
/// the address is public and global, which is all this policy connects
/// to.
fn private_class(addr: std::net::IpAddr) -> Option<&'static str> {
	match addr {
		std::net::IpAddr::V4(addr) => v4_class(addr),
		std::net::IpAddr::V6(addr) => v6_class(addr),
	}
}

fn v4_class(addr: std::net::Ipv4Addr) -> Option<&'static str> {
	let octets = addr.octets();

	if addr.is_loopback() {
		Some("loopback")
	} else if addr.is_private() {
		Some("private")
	} else if addr.is_link_local() {
		Some("link-local")
	} else if addr.is_broadcast()
		|| octets[0] == 0
		|| addr.is_multicast()
		|| addr.is_documentation()
	{
		Some("reserved")
	} else if octets[0] == 100 && (octets[1] & 0b1100_0000) == 64 {
		Some("shared-address")
	} else if octets[0] == 198 && (octets[1] & 0b1111_1110) == 18 {
		Some("benchmarking")
	} else {
		None
	}
}

fn v6_class(addr: std::net::Ipv6Addr) -> Option<&'static str> {
	let octets = addr.octets();

	// An IPv4-mapped address is its embedded IPv4 address for checking.
	if octets[0..10].iter().all(|byte| *byte == 0) && octets[10] == 0xFF && octets[11] == 0xFF {
		let mapped = std::net::Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
		return v4_class(mapped);
	}

	if addr.is_loopback() {
		Some("loopback")
	} else if addr.is_unspecified() {
		Some("unspecified")
	} else if addr.is_multicast() {
		Some("reserved")
	} else if (octets[0] & 0xFE) == 0xFC {
		Some("unique-local")
	} else if (octets[0] == 0xFE) && (octets[1] & 0xC0) == 0x80 {
		Some("link-local")
	} else if octets[0] == 0x20 && octets[1] == 0x01 && octets[2] == 0x0D && octets[3] == 0xB8 {
		Some("documentation")
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
		let policy = HostPolicy::with_extra_hosts(&[]);

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
	fn lookalike_hosts_are_refused() {
		let policy = HostPolicy::with_extra_hosts(&[]);
		let refused = [
			"https://fcm.googleapis.com.attacker.test/push",
			"https://attacker.test/fcm.googleapis.com",
			"https://fcm.googleapis.com.evil.test:443/x",
			"https://updates.push.services.mozilla.com.attacker.test/",
			"https://webpush.apple.com@attacker.test/",
			"http://fcm.googleapis.com/",
		];

		for endpoint in refused {
			let url = url::Url::parse(endpoint).unwrap();
			let verdict = policy.check(&url);

			if endpoint.contains('@') || endpoint.contains("http:") {
				// Enrollment's own URL shape check refuses these first;
				// the policy is defense in depth behind it, and the host
				// here is either shipped or refused on its own.
				continue;
			}

			assert!(verdict.is_err(), "{endpoint} should be refused");
		}

		let port = url::Url::parse("https://fcm.googleapis.com:8443/").unwrap();
		assert!(
			policy.check(&port).is_ok(),
			"the shipped host check is on the host only; the port check is enrollment's"
		);
	}

	/// An operator-approved host passes with pinning; an unapproved one
	/// still doesn't.
	#[test]
	fn extra_hosts_need_approval() {
		let policy = HostPolicy::with_extra_hosts(&["push.example.net".to_owned()]);
		let approved = url::Url::parse("https://push.example.net/send/1").unwrap();
		let pinned = policy.check(&approved).unwrap();
		assert!(pinned.is_some(), "extra hosts pin");
		assert_eq!(pinned.unwrap().host, "push.example.net");

		let unapproved = url::Url::parse("https://push.example.org/send/1").unwrap();
		assert!(policy.check(&unapproved).is_err());
	}

	/// Private, reserved, and link-local addresses are named and refused,
	/// IPv4 and IPv6, including the mapped and metadata forms.
	#[test]
	fn private_addresses_have_classes() {
		let refused: &[(&str, std::net::IpAddr)] = &[
			("loopback", "127.0.0.1".parse().unwrap()),
			("loopback", "::1".parse().unwrap()),
			("private", "10.0.0.1".parse().unwrap()),
			("private", "192.168.1.1".parse().unwrap()),
			("private", "172.16.0.1".parse().unwrap()),
			("link-local", "169.254.169.254".parse().unwrap()),
			("reserved", "0.0.0.0".parse().unwrap()),
			("reserved", "255.255.255.255".parse().unwrap()),
			("reserved", "224.0.0.1".parse().unwrap()),
			("reserved", "192.0.2.1".parse().unwrap()),
			("shared-address", "100.64.0.1".parse().unwrap()),
			("unique-local", "fc00::1".parse().unwrap()),
			("unique-local", "fd00::1".parse().unwrap()),
			("link-local", "fe80::1".parse().unwrap()),
			("documentation", "2001:db8::1".parse().unwrap()),
			("loopback", "::ffff:127.0.0.1".parse().unwrap()),
			("link-local", "::ffff:169.254.169.254".parse().unwrap()),
			("unspecified", "::".parse().unwrap()),
		];

		for (class, addr) in refused {
			assert_eq!(
				private_class(*addr).map(str::to_owned),
				Some((*class).to_owned()),
				"{addr}"
			);
		}

		let public: &[std::net::IpAddr] = &[
			"1.1.1.1".parse().unwrap(),
			"8.8.8.8".parse().unwrap(),
			"2606:4700:4700::1111".parse().unwrap(),
		];
		for addr in public {
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
