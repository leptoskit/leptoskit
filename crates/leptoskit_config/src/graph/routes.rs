//! Public route ownership, independent of the edge provider.

use std::borrow::Cow;
use std::collections::BTreeMap;

use super::Unit;

/// A frontend or service's declared public route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
	pub unit: Unit,
	pub host: String,
	pub domain: String,
	/// `/` or an absolute path ending in `/`, on segment boundaries.
	pub mount: String,
}

/// Validated routes, sorted independently of declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routes {
	routes: Vec<Route>,
}

/// Resolution of a canonical request path, before reading any file or
/// forwarding to a process.
#[derive(Debug, PartialEq, Eq)]
pub enum Match<'a> {
	/// The bare mount redirects to the declared trailing-slash path. The
	/// caller preserves a separately parsed query string.
	Redirect(&'a Route),
	/// Only this owner may serve the path. A missing file must not fall
	/// through to a less specific route.
	Serve {
		route: &'a Route,
		/// Path relative to the owning mount; empty means its index.
		relative: Cow<'a, str>,
	},
}

impl Routes {
	/// Validate ownership and normalize DNS case. A hostname has one ingress
	/// host; routing its paths to different hosts requires an explicit edge
	/// gateway model rather than conflicting DNS declarations.
	///
	/// # Errors
	/// Rejects invalid names, domains or mounts, duplicate route ownership,
	/// and a hostname assigned to several ingress hosts.
	pub fn new(mut routes: Vec<Route>) -> Result<Self, String> {
		let mut domains = BTreeMap::new();
		let mut owners = BTreeMap::new();

		for route in &mut routes {
			route.unit.validate()?;

			if !super::identifier(&route.host) {
				return Err(format!(
					"{} names invalid host {:?}",
					route.unit, route.host
				));
			}

			route.domain = domain(&route.domain)?;

			if !mount(&route.mount) {
				return Err(format!(
					"{}.mount must be / or a canonical absolute path ending in /",
					route.unit
				));
			}

			if let Some(host) = domains.insert(route.domain.clone(), route.host.clone())
				&& host != route.host
			{
				return Err(format!(
					"{} has conflicting ingress hosts {host} and {}",
					route.domain, route.host
				));
			}
			if let Some(unit) = owners.insert(
				(route.domain.clone(), route.mount.clone()),
				route.unit.clone(),
			) {
				return Err(format!(
					"{unit} and {} both own {}{}",
					route.unit, route.domain, route.mount
				));
			}
		}

		routes.sort_by(|a, b| (&a.domain, &a.mount, &a.unit).cmp(&(&b.domain, &b.mount, &b.unit)));
		Ok(Self { routes })
	}

	/// Canonical declarations for a plan, edge renderer or release descriptor.
	#[must_use]
	pub fn entries(&self) -> &[Route] {
		&self.routes
	}

	/// Find the most specific owner. The caller supplies a parsed hostname
	/// and a URI path without its query. Percent escapes are decoded once;
	/// encoded separators and noncanonical paths are rejected. A complete
	/// URL or Host header with a port is not a hostname.
	///
	/// # Errors
	/// Ambiguous paths and malformed hostnames fail before any fallback route
	/// can be selected. `None` means the project owns no matching route.
	pub fn resolve<'a>(
		&'a self,
		hostname: &str,
		path: &'a str,
	) -> Result<Option<Match<'a>>, String> {
		let hostname = domain(hostname)?;
		let path = decode_path(path)?;
		let route = self
			.routes
			.iter()
			.filter(|route| route.domain == hostname)
			.filter(|route| {
				path.starts_with(&route.mount)
					|| (route.mount != "/" && path.as_ref() == route.mount.trim_end_matches('/'))
			})
			.max_by_key(|route| route.mount.len());

		let Some(route) = route else {
			return Ok(None);
		};

		if path.len() < route.mount.len() {
			return Ok(Some(Match::Redirect(route)));
		}

		let relative = match path {
			Cow::Borrowed(path) => Cow::Borrowed(&path[route.mount.len()..]),
			Cow::Owned(path) => Cow::Owned(path[route.mount.len()..].to_owned()),
		};

		Ok(Some(Match::Serve { route, relative }))
	}
}

fn domain(value: &str) -> Result<String, String> {
	let value = value.strip_suffix('.').unwrap_or(value);

	if value.len() > 253
		|| !value.contains('.')
		|| value.split('.').any(|label| {
			label.is_empty()
				|| label.len() > 63
				|| label.starts_with('-')
				|| label.ends_with('-')
				|| !label
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b == b'-')
		}) {
		return Err(format!(
			"{value:?} must be a DNS hostname without a scheme, port or wildcard"
		));
	}
	Ok(value.to_ascii_lowercase())
}

fn mount(value: &str) -> bool {
	value.ends_with('/') && !value.contains(['%', '?', '#']) && request_path(value)
}

fn request_path(value: &str) -> bool {
	if value == "/" {
		return true;
	}

	let Some(path) = value.strip_prefix('/') else {
		return false;
	};
	let path = path.strip_suffix('/').unwrap_or(path);
	super::components(path)
}

fn decode_path(value: &str) -> Result<Cow<'_, str>, String> {
	let problem = || {
		"route lookup requires a canonical URI path without a query or encoded separator".to_owned()
	};

	if value.contains(['?', '#']) {
		return Err(problem());
	}

	if !value.contains('%') {
		return request_path(value)
			.then_some(Cow::Borrowed(value))
			.ok_or_else(problem);
	}

	let mut decoded = Vec::with_capacity(value.len());
	let mut bytes = value.bytes();

	while let Some(byte) = bytes.next() {
		if byte != b'%' {
			decoded.push(byte);
			continue;
		}

		let high = bytes
			.next()
			.and_then(|b| char::from(b).to_digit(16))
			.ok_or_else(problem)?;
		let low = bytes
			.next()
			.and_then(|b| char::from(b).to_digit(16))
			.ok_or_else(problem)?;
		let byte = u8::try_from(high * 16 + low).map_err(|_| problem())?;

		if byte == b'/' || byte == b'\\' {
			return Err(problem());
		}

		decoded.push(byte);
	}

	let decoded = String::from_utf8(decoded).map_err(|_| problem())?;

	if !request_path(&decoded) {
		return Err(problem());
	}

	Ok(Cow::Owned(decoded))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn route(name: &str, path: &str) -> Route {
		Route {
			unit: Unit::Frontend(name.into()),
			host: "web".into(),
			domain: "leptoskit.dev".into(),
			mount: path.into(),
		}
	}

	fn website() -> Routes {
		Routes::new(vec![
			route("site", "/"),
			route("docs", "/docs/"),
			route("play", "/play/"),
		])
		.unwrap()
	}

	#[test]
	fn website_routes_are_independent_and_do_not_swallow_segment_neighbors() {
		let routes = website();
		for (path, owner, relative) in [
			("/", "site", ""),
			("/docs/", "docs", ""),
			("/docs/guide/a.html", "docs", "guide/a.html"),
			("/docs2/a", "site", "docs2/a"),
			("/play/pkg/chunk.wasm", "play", "pkg/chunk.wasm"),
		] {
			let Some(Match::Serve {
				route,
				relative: actual,
			}) = routes.resolve("LEPTOSKIT.DEV.", path).unwrap()
			else {
				panic!("{path}");
			};
			assert_eq!(route.unit.name(), owner);
			assert_eq!(actual, relative);
		}
		assert!(routes.resolve("other.dev", "/docs/").unwrap().is_none());
	}

	#[test]
	fn bare_mount_redirects_and_unknown_child_asset_stays_with_its_owner() {
		let routes = website();
		let Some(Match::Redirect(route)) = routes.resolve("leptoskit.dev", "/docs").unwrap() else {
			panic!();
		};
		assert_eq!(route.mount, "/docs/");
		let Some(Match::Serve { route, .. }) = routes
			.resolve("leptoskit.dev", "/docs/missing.wasm")
			.unwrap()
		else {
			panic!();
		};
		assert_eq!(route.unit.name(), "docs");
	}

	#[test]
	fn declaration_order_does_not_change_the_plan_or_lookup() {
		let routes = website();
		let mut input = routes.entries().to_vec();
		input.reverse();
		assert_eq!(Routes::new(input).unwrap(), routes);
	}

	#[test]
	fn duplicate_ownership_including_case_aliases_is_rejected() {
		let mut second = route("other", "/docs/");
		second.domain = "LEPTOSKIT.DEV.".into();
		assert!(
			Routes::new(vec![route("docs", "/docs/"), second])
				.unwrap_err()
				.contains("both own")
		);
		assert!(Routes::new(vec![route("docs", "/docs/"), route("docs", "/docs/")]).is_err());
	}

	#[test]
	fn dns_cannot_silently_choose_one_of_two_hosts() {
		let mut second = route("docs", "/docs/");
		second.host = "compiler".into();
		assert!(
			Routes::new(vec![route("site", "/"), second])
				.unwrap_err()
				.contains("conflicting ingress")
		);
	}

	#[test]
	fn subdomain_service_is_independent_of_main_site() {
		let mut input = website().entries().to_vec();
		input.push(Route {
			unit: Unit::Service("play-api".into()),
			host: "compiler".into(),
			domain: "play-api.leptoskit.dev".into(),
			mount: "/".into(),
		});
		let routes = Routes::new(input).unwrap();
		let Some(Match::Serve { route, relative }) = routes
			.resolve("play-api.leptoskit.dev", "/compile")
			.unwrap()
		else {
			panic!();
		};
		assert_eq!(route.unit, Unit::Service("play-api".into()));
		assert_eq!(relative, "compile");
	}

	#[test]
	fn ambiguous_request_paths_cannot_fall_back_to_the_root() {
		let routes = website();
		for path in [
			"",
			"docs/",
			"//docs/",
			"/docs//file",
			"/docs/../play/",
			"/docs/./a",
			"/docs%2ffile",
			"/docs\\file",
			"/docs/?x=y",
			"/docs/#part",
			"/docs/\0file",
		] {
			assert!(routes.resolve("leptoskit.dev", path).is_err(), "{path:?}");
		}
	}

	#[test]
	fn encoded_members_keep_their_owner_and_are_decoded_exactly_once() {
		let routes = website();
		for (path, relative) in [
			("/%64ocs/My%20file.html", "My file.html"),
			("/docs/%C3%9Cberblick.html", "Überblick.html"),
			("/docs/100%25done.txt", "100%done.txt"),
			("/docs/%252e%252e.txt", "%2e%2e.txt"),
			("/docs/why%3F.txt", "why?.txt"),
		] {
			let Some(Match::Serve {
				route,
				relative: actual,
			}) = routes.resolve("leptoskit.dev", path).unwrap()
			else {
				panic!();
			};
			assert_eq!(route.unit.name(), "docs");
			assert_eq!(actual, relative);
		}
		for path in [
			"/docs/%",
			"/docs/%GG",
			"/docs/%FF",
			"/docs/%00",
			"/docs/%2e%2e/play",
			"/docs%2Fplay/",
			"/docs/%5cfile",
		] {
			assert!(routes.resolve("leptoskit.dev", path).is_err(), "{path}");
		}
	}

	#[test]
	fn malformed_declarations_are_rejected_before_rendering() {
		for path in ["docs/", "/docs", "/docs//", "/../", "/%64ocs/", "/docs/?/"] {
			assert!(Routes::new(vec![route("docs", path)]).is_err(), "{path}");
		}
		for hostname in [
			"localhost",
			"*.leptoskit.dev",
			"https://leptoskit.dev",
			"leptoskit.dev:443",
			"leptoskit..dev",
			"leptoskit.dev..",
			"-a.dev",
			"a_.dev",
		] {
			let mut declaration = route("docs", "/");
			declaration.domain = hostname.into();
			assert!(Routes::new(vec![declaration]).is_err(), "{hostname}");
		}
	}
}
