//! The service's environment: where it binds, where its state lives, and
//! the two public origins it answers for.
//!
//! Everything here is validated before the runtime or the listener exist:
//! a misconfigured service refuses to start instead of coming up half
//! wrong. The bind stays on private loopback — the edge terminates the
//! public origins and forwards to this process — and HTTPS is required
//! unless both hosts are loopback or localhost development names.

use std::fmt;
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
	/// Normalizes a `Host` header value (`host`, `host:port`, or
	/// `HOST.`): lowercase, default ports folded in, trailing dot dropped.
	///
	/// IPv6 literals arrive bracketed; the brackets are kept so the
	/// authority stays printable as it arrived.
	#[must_use]
	pub fn of_host_header(value: &str) -> Option<Self> {
		let value = value.trim().trim_end_matches('.').to_ascii_lowercase();
		if value.is_empty() {
			return None;
		}

		let (host, port) = if value.starts_with('[') {
			let (host, rest) = value.split_once(']')?;
			let host = format!("{host}]");
			let port = rest
				.strip_prefix(':')
				.and_then(|port| port.parse::<u16>().ok());
			(host, port)
		} else {
			match value.rsplit_once(':') {
				Some((host, port)) => {
					let Ok(port) = port.parse::<u16>() else {
						// A colon that isn't a port (an unbracketed IPv6
						// literal) is not an authority this service
						// answers for.
						return None;
					};
					(host.to_owned(), Some(port))
				}
				None => (value, None),
			}
		};

		Some(Self { host, port })
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

	/// An origin's authority: its lowercase host with the port the scheme
	/// implies made explicit. IPv6 hosts keep the brackets a `Host` header
	/// carries.
	fn of_origin(origin: &Url) -> Result<Self> {
		let host = match origin.host() {
			Some(url::Host::Ipv6(address)) => format!("[{address}]"),
			Some(url::Host::Ipv4(address)) => address.to_string(),
			Some(url::Host::Domain(domain)) => domain.to_ascii_lowercase(),
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
	/// The full origins, as they were configured.
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
		ensure!(
			bind.ip().is_loopback(),
			"DEMO_BIND must stay on private loopback; the edge terminates the public origins"
		);

		let volume = std::path::PathBuf::from(read("LEPTOSKIT_SERVICE_VOLUME_STATE")?);
		let (site_origin, site) = origin(read("DEMO_SITE_ORIGIN")?.as_str())
			.context("DEMO_SITE_ORIGIN is not a usable origin")?;
		let (admin_origin, admin) = origin(read("DEMO_ADMIN_ORIGIN")?.as_str())
			.context("DEMO_ADMIN_ORIGIN is not a usable origin")?;

		let bind_authority = Authority {
			host: bind.ip().to_string(),
			port: Some(bind.port()),
		};
		ensure!(
			site != admin && site != bind_authority && admin != bind_authority,
			"the site origin, the admin origin, and the bind must be distinct authorities"
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
/// or credentials; HTTPS, unless the host is loopback or a localhost
/// development name, where HTTP is accepted.
fn origin(value: &str) -> Result<(String, Authority)> {
	let parsed = Url::parse(value).with_context(|| format!("{value} is not a URL"))?;
	let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
	let loopback = host == "localhost"
		|| host.ends_with(".localhost")
		|| parsed.host().is_some_and(|host| {
			matches!(host, url::Host::Ipv4(address) if address.is_loopback())
				|| matches!(host, url::Host::Ipv6(address) if address.is_loopback())
		});
	ensure!(
		parsed.scheme() == "https" || loopback,
		"{value} must be an HTTPS origin; HTTP is only accepted for loopback or localhost \
		 development"
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

	let trimmed = value.trim_end_matches('/');
	Ok((trimmed.to_owned(), Authority::of_origin(&parsed)?))
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
		assert_eq!(text, "https://Notes.Example");
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

	// SAFETY: this test owns its process environment; no other test in
	// this crate reads these variables, and tests run on one thread
	// each.
	#[expect(unsafe_code, reason = "the test process's own environment")]
	fn environment(bind: &str, volume: &str, site: &str, admin: &str) {
		unsafe {
			std::env::set_var("DEMO_BIND", bind);
			std::env::set_var("LEPTOSKIT_SERVICE_VOLUME_STATE", volume);
			std::env::set_var("DEMO_SITE_ORIGIN", site);
			std::env::set_var("DEMO_ADMIN_ORIGIN", admin);
		}
	}

	#[test]
	fn the_environment_configures_the_whole_service() {
		environment(
			"127.0.0.1:29599",
			"/tmp/demo-config-env-test",
			"http://localhost",
			"http://127.0.0.1",
		);
		let config = Config::from_env().expect("the environment parses");
		assert_eq!(config.authorities.site.host, "localhost");
		assert_eq!(config.authorities.site.port, Some(80));
		assert_eq!(config.authorities.admin.host, "127.0.0.1");
		assert_eq!(config.authorities.admin.port, Some(80));
		assert_eq!(config.authorities.bind.port, Some(29_599));

		for host in ["localhost", "LOCALHOST", "127.0.0.1"] {
			let request = Authority::of_host_header(host).expect("host");
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
