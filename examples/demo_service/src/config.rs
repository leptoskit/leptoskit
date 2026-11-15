//! The service's environment: where it binds, where its state lives, and
//! the two public origins it answers for.
//!
//! Everything here is validated before the runtime or the listener exist:
//! a misconfigured service refuses to start instead of coming up half
//! wrong. The bind stays on private loopback — the edge terminates the
//! public origins and forwards to this process — and HTTPS is required
//! unless both hosts are loopback or localhost development names.

use std::fmt;
use std::net::Ipv6Addr;
use std::net::SocketAddr;
use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use url::Url;

/// The two origins and the private bind, each as the normalized authority
/// a request's `Host` header must name exactly.
#[derive(Clone, Debug)]
pub struct Authorities {
	pub site: Authority,
	pub admin: Authority,
	pub bind: Authority,
}

impl Authorities {
	/// Whether a request's authority names one of this service's.
	#[must_use]
	pub fn allows(&self, authority: &Authority) -> bool {
		self.site.matches(authority)
			|| self.admin.matches(authority)
			|| self.bind.matches(authority)
	}
}

/// A lowercase host with its port, or without one: how a `Host` header
/// is compared with a configured origin, consistently on every path. A
/// configured origin always knows its port (explicit or the scheme's
/// default); a `Host` header carries a port only when the request wrote
/// one.
#[derive(Clone, PartialEq, Eq)]
pub struct Authority {
	pub host: String,
	pub port: Option<u16>,
}

impl Authority {
	/// Parses a `Host` header value (`host`, `host:port`, or `HOST.:443`)
	/// into the authority a configured origin is compared against, or
	/// `None` for anything that cannot mean one: empty or ambiguous
	/// values, unbracketed IPv6, userinfo, whitespace, delimiters,
	/// repeated trailing dots, and missing, zero, or invalid ports.
	///
	/// IPv6 literals must arrive bracketed and are canonicalized, so
	/// `[0:0:0:0:0:0:0:1]` and `[::1]` are the same host, exactly as the
	/// URL parser spells a configured IPv6 origin.
	#[must_use]
	pub fn of_host_header(value: &str) -> Option<Self> {
		let value = value.trim();
		if value.is_empty() {
			return None;
		}
		if let Some(inner) = value.strip_prefix('[') {
			return Self::bracketed(inner);
		}

		let (host, port) = match value.rsplit_once(':') {
			Some((host, port)) => (host, Some(parse_port(port)?)),
			None => (value, None),
		};
		let host = normalize_host(host)?;
		Some(Self { host, port })
	}

	/// The bracketed part of an IPv6 Host header: `::1]` or
	/// `::1]:8080`, canonicalized. Anything after the closing bracket
	/// but a port, and any port a request cannot have written — empty,
	/// zero, or not a `u16` — names no authority.
	fn bracketed(inner: &str) -> Option<Self> {
		let (literal, rest) = inner.split_once(']')?;
		// After the closing bracket, only a port may follow.
		let port = match rest {
			"" => None,
			rest => Some(parse_port(rest.strip_prefix(':')?)?),
		};
		let address = literal.parse::<Ipv6Addr>().ok()?;
		Some(Self {
			host: format!("[{address}]"),
			port,
		})
	}

	/// Whether a request's authority names this one: the host must be
	/// exact, and a request that wrote a port must have written this
	/// one. A bare `Host` — what an edge that terminated TLS forwards —
	/// names this origin when its port is the scheme's default.
	#[must_use]
	pub fn matches(&self, request: &Authority) -> bool {
		self.host == request.host
			&& match request.port {
				Some(port) => self.port == Some(port),
				None => matches!(self.port, Some(80 | 443)),
			}
	}

	/// An origin's authority: its host, normalized the same way a Host
	/// header is — lowercase, one trailing dot folded away, IPv4 plain,
	/// IPv6 canonically bracketed — with the port the scheme implies
	/// made explicit.
	fn of_origin(origin: &Url) -> Result<Self> {
		let host = match origin.host() {
			Some(url::Host::Ipv6(address)) => format!("[{address}]"),
			Some(url::Host::Ipv4(address)) => address.to_string(),
			Some(url::Host::Domain(domain)) => {
				normalize_host(domain).context("the origin's host is not a usable domain name")?
			}
			None => anyhow::bail!("the origin has no host"),
		};
		let port = origin
			.port_or_known_default()
			.context("the origin's scheme has no default port")?;

		Ok(Self {
			host,
			port: Some(port),
		})
	}

	/// A socket address's authority: the same spelling a Host header
	/// carries for the same address — plain IPv4, canonically bracketed
	/// IPv6 — so the bind, the origins, and incoming requests compare
	/// equal or distinct without spelling accidents.
	fn of_bind(bind: SocketAddr) -> Self {
		let host = match bind.ip() {
			std::net::IpAddr::V4(address) => address.to_string(),
			std::net::IpAddr::V6(address) => format!("[{address}]"),
		};
		Self {
			host,
			port: Some(bind.port()),
		}
	}
}

impl fmt::Display for Authority {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self.port {
			Some(port) => write!(f, "{}:{port}", self.host),
			None => write!(f, "{}", self.host),
		}
	}
}

impl fmt::Debug for Authority {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{self}")
	}
}

/// The validated environment.
#[derive(Clone, Debug)]
pub struct Config {
	/// The private loopback address the one listener binds.
	pub bind: SocketAddr,
	/// The state root: mode 0700, owned by this account, not a symlink.
	pub volume: std::path::PathBuf,
	/// The configured origins in their normalized spelling.
	pub site_origin: String,
	pub admin_origin: String,
	/// Their authorities, plus the bind's.
	pub authorities: Authorities,
}

impl Config {
	/// Reads and validates the environment. Called before the runtime and
	/// the listener exist.
	///
	/// # Errors
	///
	/// When a variable is missing or any value fails its checks.
	pub fn from_env() -> Result<Self> {
		let read = |name: &str| -> Result<String> {
			std::env::var(name)
				.ok()
				.map(|value| value.trim().to_owned())
				.filter(|value| !value.is_empty())
				.with_context(|| format!("{name} must be set"))
		};

		let bind: SocketAddr = read("DEMO_BIND")?
			.parse()
			.context("DEMO_BIND must be an address such as 127.0.0.1:8080")?;
		let volume = std::path::PathBuf::from(read("LEPTOSKIT_SERVICE_VOLUME_STATE")?);
		let site = read("DEMO_SITE_ORIGIN")?;
		let admin = read("DEMO_ADMIN_ORIGIN")?;
		Self::parse(bind, volume, &site, &admin)
	}

	/// Validates the parsed inputs into a configuration. The narrow seam
	/// the unit tests use instead of mutating a process-global
	/// environment: the environment is read only in `from_env`, and every
	/// check here — the bind's loopback rule, the origins, and the three
	/// authorities' distinctness after normalization — runs before the
	/// runtime and the listener exist.
	///
	/// # Errors
	///
	/// When the bind is off loopback, either origin fails its checks, or
	/// two of the three authorities fold into one.
	fn parse(
		bind: SocketAddr,
		volume: std::path::PathBuf,
		site: &str,
		admin: &str,
	) -> Result<Self> {
		ensure!(
			bind.ip().is_loopback(),
			"the bind must stay on private loopback; the edge terminates the public origins"
		);
		let (site_origin, site) = origin(site).context("the site origin is not a usable origin")?;
		let (admin_origin, admin) =
			origin(admin).context("the admin origin is not a usable origin")?;

		let bind_authority = Authority::of_bind(bind);
		ensure!(
			site != admin && site != bind_authority && admin != bind_authority,
			"the site origin, the admin origin, and the bind must be distinct authorities; after \
			 normalization two of them name the same one"
		);

		Ok(Self {
			bind,
			volume,
			site_origin,
			admin_origin,
			authorities: Authorities {
				site,
				admin,
				bind: bind_authority,
			},
		})
	}

	/// The state root's checks: a real directory, mode 0700, owned by this
	/// account, and not a symlink — a symlinked root would let the state
	/// live somewhere these checks never saw.
	///
	/// # Errors
	///
	/// When the root is missing, a symlink, not owned by the effective
	/// uid, or has permissions other than 0700.
	pub fn check_state_root(&self) -> Result<()> {
		let root: &Path = &self.volume;
		let metadata = std::fs::symlink_metadata(root)
			.with_context(|| format!("the state root {} is not usable", root.display()))?;
		ensure!(
			!metadata.file_type().is_symlink(),
			"the state root {} must not be a symlink",
			root.display()
		);
		ensure!(
			metadata.is_dir(),
			"the state root {} must be a directory",
			root.display()
		);

		#[cfg(unix)]
		{
			use std::os::unix::fs::MetadataExt as _;
			use std::os::unix::fs::PermissionsExt as _;

			ensure!(
				metadata.uid() == rustix::process::geteuid().as_raw(),
				"the state root {} must be owned by the service account (uid {})",
				root.display(),
				rustix::process::geteuid()
			);
			ensure!(
				metadata.permissions().mode() & 0o7777 == 0o700,
				"the state root {} must be mode 0700, not {:o}",
				root.display(),
				metadata.permissions().mode() & 0o7777
			);
		}

		Ok(())
	}
}

/// Parses and validates a bare origin: no path, query, fragment, port 0,
/// or credentials; exactly HTTPS, or HTTP when the host is a loopback IP
/// or a localhost development name. No other scheme is accepted, even on
/// loopback. Returns the origin's normalized spelling and its authority.
fn origin(value: &str) -> Result<(String, Authority)> {
	let parsed = Url::parse(value).with_context(|| format!("{value} is not a URL"))?;
	let scheme = parsed.scheme();
	let loopback = parsed.host().is_some_and(|host| {
		match host {
			url::Host::Ipv4(address) => address.is_loopback(),
			url::Host::Ipv6(address) => address.is_loopback(),
			url::Host::Domain(domain) => {
				normalize_host(domain)
					.is_some_and(|host| host == "localhost" || host.ends_with(".localhost"))
			}
		}
	});
	ensure!(
		scheme == "https" || (scheme == "http" && loopback),
		"{value} must be an HTTPS origin; HTTP is only accepted for loopback or localhost \
		 development, and no other scheme is accepted anywhere"
	);
	ensure!(
		parsed.path() == "/" && parsed.query().is_none() && parsed.fragment().is_none(),
		"{value} must be a bare origin: no path, query, or fragment"
	);
	ensure!(
		parsed.username().is_empty() && parsed.password().is_none(),
		"{value} must not carry credentials"
	);
	ensure!(parsed.port() != Some(0), "{value} must not use port 0");

	let authority = Authority::of_origin(&parsed)?;
	let text = match (scheme, authority.port) {
		("https", Some(443)) | ("http", Some(80)) => format!("{scheme}://{}", authority.host),
		(scheme, Some(port)) => format!("{scheme}://{}:{port}", authority.host),
		(scheme, None) => format!("{scheme}://{}", authority.host),
	};
	Ok((text, authority))
}

/// A nonzero decimal port. Integer parsing alone also accepts a leading
/// `+`, which is not part of an HTTP authority's port grammar.
fn parse_port(value: &str) -> Option<u16> {
	if !value.bytes().all(|byte| byte.is_ascii_digit()) {
		return None;
	}

	let port = value.parse::<u16>().ok()?;
	(port != 0).then_some(port)
}

/// One host, normalized the same way on every path an authority enters
/// by — origins, Host headers, and the socket bind: lowercase, exactly
/// one DNS trailing dot folded away, IPv4 plain, IPv6 canonically
/// bracketed. Empty hosts, repeated trailing dots, and anything the URL
/// parser refuses to read as a domain or an IPv4 address name nothing.
#[must_use]
fn normalize_host(host: &str) -> Option<String> {
	let lowered = host.to_ascii_lowercase();
	if lowered.is_empty()
		|| lowered
			.bytes()
			.any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_'))
	{
		return None;
	}
	let host = lowered.strip_suffix('.').unwrap_or(lowered.as_str());
	if host.is_empty() || host.ends_with('.') {
		return None;
	}
	url::Host::parse(host).ok().map(|host| host.to_string())
}

/// Refuses to exec with inherited file descriptors, on Linux, before the
/// runtime and the listener exist: every descriptor this process keeps
/// must close at exec, or the kernel would carry yesterday's database and
/// lock files into tomorrow's image.
///
/// The enumeration's own descriptor (the open `/proc/self/fd` directory)
/// is excluded by its target, not by a guess about descriptor numbers, and
/// the standard streams stay allowed: they belong to the process image.
///
/// # Errors
///
/// When `/proc/self/fd` lists a descriptor beyond the standard streams and
/// the enumeration itself.
#[cfg(target_os = "linux")]
pub fn guard_inherited_descriptors() -> Result<()> {
	let entries = std::fs::read_dir("/proc/self/fd")
		.context("the Linux descriptor guard needs /proc/self/fd")?;
	let mut inherited = Vec::new();

	for entry in entries {
		let entry = entry.context("reading /proc/self/fd")?;
		let Ok(fd) = entry.file_name().to_string_lossy().parse::<i32>() else {
			continue;
		};
		if fd <= 2 {
			continue;
		}
		// The one descriptor this check itself opened. Its symlink names
		// the directory being enumerated, so the guard can't reject its
		// own view of the world.
		let target = std::fs::read_link(format!("/proc/self/fd/{fd}")).unwrap_or_default();
		if target.to_string_lossy().ends_with("/fd") {
			continue;
		}
		inherited.push((fd, target.to_string_lossy().into_owned()));
	}

	ensure!(
		inherited.is_empty(),
		"inherited file descriptors would survive exec: {inherited:?}; close them before starting \
		 the demo"
	);
	Ok(())
}

/// Off Linux there is no `/proc/self/fd` to check; the process tests that
/// prove descriptors don't survive exec run on Linux only, and the
/// platform requirements say so rather than pretend otherwise.
#[cfg(not(target_os = "linux"))]
pub fn guard_inherited_descriptors() -> Result<()> {
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn host_headers_normalize_like_origins() {
		let (text, authority) = origin("https://Notes.Example").expect("valid origin");
		assert_eq!(text, "https://notes.example");
		assert_eq!(authority.host, "notes.example");
		assert_eq!(authority.port, Some(443));
		// What an edge that terminated TLS forwards: a bare Host header.
		assert!(
			authority.matches(&Authority::of_host_header("notes.example").expect("host")),
			"a bare Host names the https origin"
		);
		assert!(
			authority.matches(&Authority::of_host_header("NOTES.EXAMPLE.").expect("host")),
			"case and trailing dots fold away"
		);
		assert!(
			authority.matches(&Authority::of_host_header("notes.example:443").expect("host")),
			"the explicit default port matches"
		);
		assert!(
			!authority.matches(&Authority::of_host_header("notes.example:8443").expect("host")),
			"another port is another authority"
		);

		let (_, plain) = origin("http://demo.localhost").expect("localhost http");
		assert_eq!(plain.port, Some(80));
		assert!(plain.matches(&Authority::of_host_header("demo.localhost:80").expect("h")));
		assert!(plain.matches(&Authority::of_host_header("demo.localhost").expect("h")));
		assert!(!plain.matches(&authority));
	}

	#[test]
	fn origins_are_bare_https_or_localhost_http() {
		assert!(origin("https://demo.example").is_ok());
		assert!(origin("http://demo.localhost").is_ok());
		assert!(origin("http://127.0.0.1:8080").is_ok());
		assert!(origin("http://demo.example").is_err(), "plain http host");
		assert!(origin("https://demo.example/app").is_err(), "path");
		assert!(origin("https://demo.example?x=1").is_err(), "query");
		assert!(origin("https://user@demo.example").is_err(), "credentials");
		assert!(origin("ftp://demo.example").is_err(), "scheme");
		assert!(origin("https://demo.example:0").is_err(), "port 0");
	}

	/// Exactly HTTPS, or HTTP for loopback IP or localhost development
	/// names — no other scheme, even on loopback, and the same bareness
	/// rules as before.
	#[test]
	fn origins_accept_https_and_loopback_http_only() {
		for good in [
			"https://demo.example",
			"https://DEMO.Example",
			"https://demo.example.:443",
			"https://demo.example:8443",
			"http://demo.localhost",
			"http://localhost.:8080",
			"http://127.0.0.1:8080",
			"http://[::1]:8080",
			"http://[0:0:0:0:0:0:0:1]:8080",
		] {
			assert!(origin(good).is_ok(), "{good} is a usable origin");
		}
		for bad in [
			"ftp://localhost",
			"ws://localhost",
			"wss://localhost",
			"file://localhost",
			"ftp://demo.example",
			"http://demo.example",
			"http://demo.localhost.evil",
			"https://demo.example/app",
			"https://demo.example?x=1",
			"https://demo.example#frag",
			"https://user@demo.example",
			"https://user:pass@demo.example",
			"https://demo.example:0",
			"https://demo.example..",
			"https://",
		] {
			assert!(origin(bad).is_err(), "{bad} is refused");
		}
	}

	/// One spelling per authority: the parsed URL's scheme and the
	/// normalized host decide, never the input's casing, trailing dot,
	/// or redundant default port.
	#[test]
	fn origins_normalize_to_one_spelling() {
		let (text, authority) = origin("https://Notes.Example.:443").expect("valid origin");
		assert_eq!(text, "https://notes.example", "the origin is normalized");
		assert_eq!(authority.host, "notes.example");
		assert_eq!(authority.port, Some(443));
		let (_, plain) = origin("https://notes.example").expect("valid origin");
		assert_eq!(
			authority, plain,
			"explicit or implicit default port, one authority"
		);

		let (ipv6, ipv6_authority) = origin("http://[0:0:0:0:0:0:0:1]:8080").expect("valid origin");
		assert_eq!(ipv6, "http://[::1]:8080", "the IPv6 literal is canonical");
		assert_eq!(ipv6_authority.host, "[::1]");
		assert_eq!(ipv6_authority.port, Some(8080));

		let (_, ported) = origin("https://demo.example:8443").expect("valid origin");
		assert_eq!(ported.host, "demo.example");
		assert_eq!(ported.port, Some(8443));
		assert_ne!(ported, plain, "a nondefault port is another authority");
	}

	/// The grammar a Host header must satisfy to name an authority at
	/// all: a lowercase host (one trailing dot folded away) or a
	/// canonical bracketed IPv6 literal, with a port only when it is
	/// valid and nonzero.
	#[test]
	fn host_headers_parse_a_strict_grammar() {
		for value in [
			"",
			"   ",
			"[::1]garbage",
			"[::1]:bad",
			"[::1]:+443",
			"[::1]:",
			"[::1]:0",
			"[::1]:99999",
			"[garbage]",
			"::1",
			"::1:8080",
			"user@site.example",
			"site.example:",
			"site.example:0",
			"site.example:65536",
			"site.example:-1",
			"site.example:+443",
			"site.example/a",
			"site.example?q",
			"site.example#f",
			"site.example,a.example",
			"site.example..",
			".",
			"site exam ple",
			"999.1.1.1",
		] {
			assert!(
				Authority::of_host_header(value).is_none(),
				"{value:?} names no authority"
			);
		}
		for (value, host, port) in [
			("site.example", "site.example", None),
			("SITE.EXAMPLE.", "site.example", None),
			("site.example:443", "site.example", Some(443)),
			("site.example.:443", "site.example", Some(443)),
			("demo.localhost:80", "demo.localhost", Some(80)),
			("127.0.0.1", "127.0.0.1", None),
			("127.0.0.1.:8080", "127.0.0.1", Some(8080)),
			("[::1]", "[::1]", None),
			("[::1]:8080", "[::1]", Some(8080)),
			("[0:0:0:0:0:0:0:1]:8080", "[::1]", Some(8080)),
			("[::FFFF:127.0.0.1]:8080", "[::ffff:127.0.0.1]", Some(8080)),
		] {
			let authority = Authority::of_host_header(value)
				.unwrap_or_else(|| panic!("{value} is a usable host header"));
			assert_eq!(authority.host, host, "{value}'s host");
			assert_eq!(authority.port, port, "{value}'s port");
		}
	}

	/// The parse seam: everything `from_env` does except read the
	/// environment, so the tests never mutate a process-global state.
	fn configuration(bind: &str, site: &str, admin: &str) -> Result<Config> {
		Config::parse(
			bind.parse().expect("the bind parses"),
			std::path::PathBuf::from("/tmp/demo-config-parse-test"),
			site,
			admin,
		)
	}

	#[test]
	fn parsed_inputs_configure_the_whole_service() {
		let config = configuration("127.0.0.1:29599", "http://localhost", "http://127.0.0.1")
			.expect("valid inputs");
		assert_eq!(config.authorities.site.host, "localhost");
		assert_eq!(config.authorities.site.port, Some(80));
		assert_eq!(config.authorities.admin.host, "127.0.0.1");
		assert_eq!(config.authorities.admin.port, Some(80));
		assert_eq!(config.authorities.bind.host, "127.0.0.1");
		assert_eq!(config.authorities.bind.port, Some(29_599));

		for host in [
			"localhost",
			"LOCALHOST",
			"localhost.",
			"127.0.0.1",
			"127.0.0.1.:80",
		] {
			let request = Authority::of_host_header(host).expect("a usable host");
			assert!(
				config.authorities.allows(&request),
				"{host} is one of this service's authorities"
			);
		}
		let request = Authority::of_host_header("127.0.0.1:29599").expect("host");
		assert!(config.authorities.allows(&request), "the bind itself");
		let request = Authority::of_host_header("stranger.example").expect("host");
		assert!(!config.authorities.allows(&request), "a stranger");
	}

	/// The site, the admin, and the bind must stay three authorities
	/// after normalization: casing, a trailing dot, an explicit default
	/// port, and a bracketed IPv6 literal all fold before the check.
	#[test]
	fn authorities_stay_distinct_after_normalization() {
		assert!(
			configuration(
				"127.0.0.1:8080",
				"https://site.example",
				"https://SITE.example.:443"
			)
			.is_err(),
			"casing and an explicit default port fold the site and the admin into one"
		);
		assert!(
			configuration("[::1]:8080", "http://[::1]:8080", "http://localhost").is_err(),
			"an IPv6 site origin and the same IPv6 bind fold into one"
		);
		assert!(
			configuration(
				"10.0.0.1:8080",
				"https://site.example",
				"https://admin.example"
			)
			.is_err(),
			"the bind stays on loopback"
		);
	}

	/// IPv6 is canonical everywhere: the bind, an IPv6 origin, and every
	/// bracketed spelling of the same literal compare as one authority.
	#[test]
	fn ipv6_authorities_are_canonical_everywhere() {
		let config =
			configuration("[::1]:29599", "http://localhost", "http://[::1]").expect("valid inputs");
		assert_eq!(config.authorities.bind.host, "[::1]");
		assert_eq!(config.authorities.admin.host, "[::1]");
		assert_eq!(config.authorities.admin.port, Some(80));

		for host in ["[::1]:29599", "[0:0:0:0:0:0:0:1]:29599"] {
			let request = Authority::of_host_header(host).expect("a usable host");
			assert!(config.authorities.allows(&request), "{host} names the bind");
		}
		let request = Authority::of_host_header("[::1]").expect("host");
		assert!(
			config.authorities.allows(&request),
			"the bare literal names the admin's default port"
		);
		assert!(
			!config
				.authorities
				.allows(&Authority::of_host_header("[::2]").expect("host")),
			"another loopback literal is another host"
		);
	}

	#[test]
	fn distinct_authorities() {
		let (_, site) = origin("https://site.example").expect("site origin");
		let (_, admin) = origin("https://admin.example").expect("admin origin");
		let bind = Authority {
			host: "127.0.0.1".into(),
			port: Some(8080),
		};
		let config = Config {
			bind: "127.0.0.1:8080".parse().expect("bind"),
			volume: std::path::PathBuf::from("/tmp/demo-authorities-test"),
			site_origin: "https://site.example".into(),
			admin_origin: "https://admin.example".into(),
			authorities: Authorities { site, admin, bind },
		};
		assert!(
			config
				.authorities
				.allows(&Authority::of_host_header("SITE.example").expect("x")),
			"a bare Host names the site"
		);
		assert!(
			!config
				.authorities
				.allows(&Authority::of_host_header("other.example").expect("y"))
		);
	}
}
