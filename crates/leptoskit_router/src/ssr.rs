//! Server-rendered apps: file routes in step with [`leptos_axum`] and the
//! request URL.
//!
//! [`leptos_axum`]: https://docs.rs/leptos_axum

use leptos::prelude::use_context;
use leptos_axum::AxumRouteListing;
use leptos_router::Method;
use leptos_router::SsrMode;
use leptos_router::location::RequestUrl;

use crate::NoMatch;
use crate::Routable;
use crate::Router;

/// Builds the route table [`leptos_axum::LeptosRoutes::leptos_routes`] expects
/// from the paths `leptoskit routes` wrote in [`axum_route_list`].
pub fn route_listings(paths: impl IntoIterator<Item = impl Into<String>>) -> Vec<AxumRouteListing> {
	paths.into_iter().map(route_listing).collect()
}

/// One SSR path every method in `paths` should answer with the app shell.
pub fn route_listing(path: impl Into<String>) -> AxumRouteListing {
	AxumRouteListing::new(path.into(), SsrMode::Async, [Method::Get], [])
}

/// A router for the request being rendered, when [`leptos_axum`] provided a
/// [`RequestUrl`] in context.
pub fn navigation_from_request<R>() -> Result<Router<R>, NoMatch>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	let request = use_context::<RequestUrl>().ok_or(NoMatch)?;
	let path = request.as_ref();
	// `RequestUrl` may carry the full origin; the file router matches on path
	// and query only.
	let path = request
		.parse()
		.map(|url| {
			if url.search().is_empty() {
				url.path().to_owned()
			} else {
				format!("{}?{}", url.path(), url.search())
			}
		})
		.unwrap_or_else(|_| path.to_owned());
	Ok(Router::new(R::parse(&path)?))
}
