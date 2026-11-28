//! The console's routes: every page rendered on the server, the client's
//! Wasm and its loader, and the two faces the design uses.
//!
//! The pages sit beside the control plane's routes and are layered the same
//! way (`super::router`), the session layer inside the OAuth provider's, so
//! a page reads the same session an operation would. Rendering reads
//! sign-in and the device flow in process through those layers' request
//! extensions.

use std::path::Path;
use std::path::PathBuf;

use leptos::config::LeptosOptions;
use leptos::prelude::provide_context;
use leptoskit::server::axum::Router;
use leptoskit::server::axum::http::HeaderValue;
use leptoskit::server::axum::http::header;
use leptoskit::server::axum::response::IntoResponse;
use leptoskit::server::axum::routing::get;
use leptoskit::server::leptos_axum::LeptosRoutes as _;

use crate::app::shell;
use crate::routes;
use crate::style;

/// Where the client's files are served from: under `/_leptoskit/`, which no
/// organisation slug can start with.
pub const PKG_PATH: &str = "_leptoskit/pkg";

/// The client library's name: the stem of `pkg/leptoskit_cloud_app.js`.
pub const OUTPUT_NAME: &str = "leptoskit_cloud_app";

const MONO: &[u8] = include_bytes!("../../assets/fonts/SkribbleMonoPlayful-Latin.woff2");
const HAND: &[u8] = include_bytes!("../../assets/fonts/SkribbleNotoSketch-Latin.woff2");

/// The leptos options the console renders with. `site` is the directory the
/// client build wrote, holding `pkg/`.
#[must_use]
pub fn options(site: Option<&Path>) -> LeptosOptions {
	LeptosOptions::builder()
		.output_name(OUTPUT_NAME)
		.site_root(site.map_or_else(
			|| "target/site".to_owned(),
			|site| site.display().to_string(),
		))
		.site_pkg_dir(PKG_PATH)
		.build()
}

/// The console's routes, unlayered: the binary layers them with the rest of
/// its own routes (`super::router`). `site` holds the client build's
/// `pkg/`; without one, pages render but don't hydrate.
pub fn routes(site: Option<PathBuf>) -> Router {
	let options = options(site.as_deref());
	// The not-found page's catch-all can't sit beside `/{org}` in axum's
	// route table, so any address no page claims falls back to rendering,
	// and the router answers it with the not-found page and a 404.
	let pages: Vec<String> = routes::axum_route_list()
		.iter()
		.map(|path| axum_path(path))
		.filter(|path| !path.contains("{*"))
		.collect();
	let render = leptoskit::server::leptos_axum::render_app_async_with_context(
		{
			let options = options.clone();
			move || provide_context(options.clone())
		},
		{
			let options = options.clone();
			move || shell(options.clone())
		},
	);
	let mut router = Router::<LeptosOptions>::new()
		.leptos_routes(&options, leptoskit_router::ssr::route_listings(pages), {
			let options = options.clone();
			move || shell(options.clone())
		})
		.fallback(render)
		.route(
			&format!("{}/{}", style::FONT_PATH, style::MONO_FONT),
			get(|| async { font(MONO) }),
		)
		.route(
			&format!("{}/{}", style::FONT_PATH, style::HAND_FONT),
			get(|| async { font(HAND) }),
		);

	if let Some(site) = site {
		router = router.nest_service(
			&format!("/{PKG_PATH}"),
			leptoskit::server::static_files(&site.join("pkg")),
		);
	}

	router
		.layer(leptoskit::server::axum::middleware::map_response(
			private_pages,
		))
		.with_state(options)
}

/// Marks every rendered page `private, no-store`: a page carries the signed-in
/// person's data (their organisations, and on the billing page its usage and
/// cap) in the query results it streams, so no shared cache may keep it.
/// Fonts and the client build keep their own caching.
///
/// A page also carries what keeps it a page of this console's: no site may
/// frame it, a browser reads it as HTML only, and it tells no other site its
/// address.
async fn private_pages(
	mut response: leptoskit::server::axum::response::Response,
) -> leptoskit::server::axum::response::Response {
	if super::pages::is_html(&response) {
		let headers = response.headers_mut();
		headers.insert(
			header::CACHE_CONTROL,
			HeaderValue::from_static("private, no-store"),
		);
		headers
			.entry(header::REFERRER_POLICY)
			.or_insert(HeaderValue::from_static("no-referrer"));
	}

	super::pages::refuse_framing(response).await
}

/// `path` with each `:name` segment written `{name}`, as axum 0.8 captures
/// them. `leptoskit_router`'s `axum_path` still writes the older `:name`,
/// which `leptos_axum` 0.9 refuses at startup.
fn axum_path(path: &str) -> String {
	path.split('/')
		.map(|segment| {
			segment
				.strip_prefix(':')
				.map_or_else(|| segment.to_owned(), |name| format!("{{{name}}}"))
		})
		.collect::<Vec<_>>()
		.join("/")
}

fn font(bytes: &'static [u8]) -> impl IntoResponse {
	(
		[
			(header::CONTENT_TYPE, HeaderValue::from_static("font/woff2")),
			(
				header::CACHE_CONTROL,
				HeaderValue::from_static("public, max-age=31536000, immutable"),
			),
		],
		bytes,
	)
}

#[cfg(test)]
mod tests {
	use super::axum_path;

	#[test]
	fn captures_are_written_for_axum() {
		assert_eq!(axum_path("/:org"), "/{org}");
		assert_eq!(axum_path("/:org/:project/new"), "/{org}/{project}/new");
		assert_eq!(axum_path("/sign-in/email"), "/sign-in/email");
		assert_eq!(axum_path("/{*path}"), "/{*path}");
		assert_eq!(axum_path("/"), "/");
	}
}
