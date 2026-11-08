//! Incoming URLs → typed routes.

use crate::NoMatch;
use crate::Routable;
use crate::Router;

/// Why [`Router::open_url`] could not navigate.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OpenError {
	/// The string was not a path or absolute URL the router understands.
	#[error("the URL is not a path or absolute link the router can open")]
	InvalidUrl,
	/// No route matched the path.
	#[error(transparent)]
	NoMatch(#[from] NoMatch),
}

/// How [`Router::open_url`] records the route in history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum OpenMode {
	/// [`Router::navigate`] — the default for a warm deep link.
	#[default]
	Navigate,
	/// [`Router::replace`] — preferred for cold start so Back leaves the app.
	Replace,
}

impl<R> Router<R>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	/// Opens an incoming URL or path: `myapp://notes/42`, `https://host/notes`,
	/// or `/notes/42`. Custom schemes treat the host as the first path
	/// segment (`myapp://notes/42` → `/notes/42`); `http(s)` uses the URL path
	/// and query only.
	pub fn open_url(&self, url: &str) -> Result<(), OpenError> {
		self.open_url_with(url, OpenMode::Navigate)
	}

	/// [`open_url`](Self::open_url) with an explicit history mode.
	pub fn open_url_with(&self, url: &str, mode: OpenMode) -> Result<(), OpenError> {
		let path = route_path_from_link(url).ok_or(OpenError::InvalidUrl)?;
		let route = R::parse(&path)?;
		match mode {
			OpenMode::Navigate => self.navigate(route),
			OpenMode::Replace => self.replace(route),
		}
		Ok(())
	}
}

/// Turns a deep link or path into the path+query `Routable::parse` expects.
pub fn route_path_from_link(input: &str) -> Option<String> {
	let input = input.trim();
	if input.is_empty() || input.chars().any(char::is_control) {
		return None;
	}

	if input.starts_with('/') {
		return Some(strip_fragment(input).to_owned());
	}

	let parsed = url::Url::parse(input).ok()?;
	let path = match parsed.scheme() {
		"http" | "https" => {
			// WHATWG http(s) URLs always have a path of at least `/`.
			let mut path = parsed.path().to_owned();
			if let Some(query) = parsed.query() {
				path.push('?');
				path.push_str(query);
			}
			path
		}
		_ => {
			let mut path = String::new();
			if let Some(host) = parsed.host_str() {
				path.push('/');
				path.push_str(host);
			}
			let url_path = parsed.path();
			if url_path != "/" && !url_path.is_empty() {
				path.push_str(url_path);
			} else if path.is_empty() {
				path.push('/');
			}
			if let Some(query) = parsed.query() {
				path.push('?');
				path.push_str(query);
			}
			path
		}
	};

	Some(path)
}

fn strip_fragment(input: &str) -> &str {
	input.split_once('#').map_or(input, |(before, _)| before)
}

/// Applies deep-link activations to a router. URL activations call
/// [`Router::open_url`]; file activations are skipped (the app owns those).
pub fn apply_activations<'a, R>(
	navigation: &Router<R>,
	activations: impl IntoIterator<Item = ActivationRef<'a>>,
	mode: OpenMode,
) -> Vec<Result<(), OpenError>>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	activations
		.into_iter()
		.filter_map(|activation| {
			match activation {
				ActivationRef::Url(url) => Some(navigation.open_url_with(url, mode)),
				ActivationRef::File(_) => None,
			}
		})
		.collect()
}

/// A borrowed deep-link activation, so this crate does not depend on the
/// `deep_link` plugin at compile time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationRef<'a> {
	/// An inbound URL.
	Url(&'a str),
	/// An inbound file path the app must handle itself.
	File(&'a std::path::Path),
}
