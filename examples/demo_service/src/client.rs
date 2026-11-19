//! The pages' browser half: the Wasm client `leptoskit build` made from
//! `demo_pages`, and the document a page is rendered into.
//!
//! The client is read once, before the listener binds, and served from
//! memory for the life of the process image. A release that replaces the
//! directory under a running service therefore changes nothing until that
//! service re-executes: the markup a process renders and the client it
//! hands out always come from the same start, which is what hydration
//! needs. Nothing here keeps a file open.
//!
//! The directory follows the layout every leptoskit server uses:
//! `LEPTOS_SITE_ROOT` is the build's `--out` directory, the client sits in
//! its `pkg/` (`LEPTOS_SITE_PKG_DIR`), and its files are named after the
//! client library (`LEPTOS_OUTPUT_NAME`, `demo_pages`).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use axum::body::Body;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse as _;
use axum::response::Response;
use demo_pages::Document;
use demo_pages::model::Page;
use leptos::config::LeptosOptions;
use leptos::prelude::provide_context;

/// The client library's name, which names its files.
pub const OUTPUT_NAME: &str = "demo_pages";

/// The directory under the site root that holds the client.
pub const PKG_DIR: &str = "pkg";

/// The most one client file may weigh. The whole client is a few hundred
/// kilobytes; anything near this is not the client.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// The most files the client directory may hold.
const MAX_FILES: usize = 64;

/// The demo's own stylesheet: only what the class compiler cannot say.
const STYLE: &str = include_str!("../assets/demo.css");

/// The faces the admin uses, served from this site so no page reaches
/// another origin for them.
const FONT_FACES: &str = r#"@font-face{font-family:"Skribble Sketch";src:url(/demo/assets/sketch.woff2) format("woff2");font-display:swap}@font-face{font-family:"Skribble Mono";src:url(/demo/assets/mono.woff2) format("woff2");font-display:swap}"#;

/// One file of the client, with the copies a release build compressed
/// ahead of time.
struct File {
	content_type: &'static str,
	plain: axum::body::Bytes,
	brotli: Option<axum::body::Bytes>,
	gzip: Option<axum::body::Bytes>,
}

/// The built client, and how a page names it.
#[derive(Clone)]
pub struct Client {
	options: LeptosOptions,
	files: Arc<BTreeMap<String, File>>,
}

impl std::fmt::Debug for Client {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Client")
			.field("files", &self.files.keys().collect::<Vec<_>>())
			.finish_non_exhaustive()
	}
}

impl Client {
	/// No client: pages render and stay as the server drew them. For the
	/// router tests, which read markup and never run a browser.
	#[must_use]
	pub fn absent() -> Self {
		Self {
			options: options(),
			files: Arc::new(BTreeMap::new()),
		}
	}

	/// Reads the client from `site_root/pkg`.
	///
	/// # Errors
	///
	/// When the directory is missing or holds something other than the
	/// client: no `demo_pages.js` or `demo_pages_bg.wasm`, a file that is
	/// not a regular file, or more or larger files than a client has.
	pub fn load(site_root: &Path) -> Result<Self> {
		let dir = site_root.join(PKG_DIR);
		let entries = std::fs::read_dir(&dir).with_context(|| {
			format!(
				"the pages' client is not at {}: build it with `leptoskit build --release \
				 --platform web` in examples/demo_service/pages and point LEPTOS_SITE_ROOT at its \
				 output",
				dir.display()
			)
		})?;
		let mut raw = BTreeMap::<String, axum::body::Bytes>::new();

		for entry in entries {
			let entry = entry.with_context(|| format!("listing {}", dir.display()))?;
			let path = entry.path();
			let metadata = std::fs::symlink_metadata(&path)
				.with_context(|| format!("inspecting {}", path.display()))?;
			ensure!(
				metadata.is_file(),
				"{} is not a regular file; the client directory holds only the built client",
				path.display()
			);
			ensure!(
				metadata.len() <= MAX_FILE_BYTES,
				"{} is larger than a client file can be",
				path.display()
			);
			let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
				anyhow::bail!("{} has a name that is not text", path.display());
			};
			ensure!(
				raw.len() < MAX_FILES,
				"{} holds more files than a client has",
				dir.display()
			);
			let bytes =
				std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
			raw.insert(name.to_owned(), bytes.into());
		}

		let mut files = BTreeMap::new();

		for (name, plain) in &raw {
			let Some(content_type) = content_type(name) else {
				continue;
			};
			files.insert(
				name.clone(),
				File {
					content_type,
					plain: plain.clone(),
					brotli: raw.get(&format!("{name}.br")).cloned(),
					gzip: raw.get(&format!("{name}.gz")).cloned(),
				},
			);
		}

		for required in [
			format!("{OUTPUT_NAME}.js"),
			format!("{OUTPUT_NAME}_bg.wasm"),
		] {
			ensure!(
				files
					.get(&required)
					.is_some_and(|file| !file.plain.is_empty()),
				"{} has no {required}; it is not the pages' built client",
				dir.display()
			);
		}

		Ok(Self {
			options: options(),
			files: Arc::new(files),
		})
	}

	/// `GET /pkg/{file}`: one file of the client, in the smallest encoding
	/// the browser accepts. The client's names carry no hash, so a browser
	/// asks again each time and a release is never served stale.
	pub fn serve(&self, name: &str, headers: &HeaderMap) -> Response {
		let Some(file) = self.files.get(name) else {
			return StatusCode::NOT_FOUND.into_response();
		};
		let accepts = |encoding: &str| {
			headers
				.get_all(header::ACCEPT_ENCODING)
				.iter()
				.filter_map(|value| value.to_str().ok())
				.flat_map(|value| value.split(','))
				.any(|offered| {
					let mut parts = offered.split(';');
					let name = parts.next().unwrap_or_default().trim();
					let refused = parts
						.any(|part| matches!(part.trim(), "q=0" | "q=0.0" | "q=0.00" | "q=0.000"));
					name.eq_ignore_ascii_case(encoding) && !refused
				})
		};
		let (bytes, encoding) = match (&file.brotli, &file.gzip) {
			(Some(brotli), _) if accepts("br") => (brotli.clone(), Some("br")),
			(_, Some(gzip)) if accepts("gzip") => (gzip.clone(), Some("gzip")),
			_ => (file.plain.clone(), None),
		};
		let mut response = (
			[
				(header::CONTENT_TYPE, file.content_type),
				(header::CACHE_CONTROL, "no-cache"),
				(header::VARY, "Accept-Encoding"),
				(header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
			],
			bytes,
		)
			.into_response();

		if let Some(encoding) = encoding {
			response
				.headers_mut()
				.insert(header::CONTENT_ENCODING, HeaderValue::from_static(encoding));
		}

		response
	}

	/// Renders `page` as a whole document: the markup the browser shows at
	/// once, the page value the client reads back, and the loader that
	/// fetches the client. `before` is plain HTML placed first in the body,
	/// already escaped, for what only a visitor without scripts sees.
	pub async fn render(&self, page: Page, before: String) -> Response {
		// The pages' clock and streams spawn their work on leptos's executor,
		// which is tokio here. Setting it twice is refused, and that is fine.
		let _ = any_spawner::Executor::init_tokio();
		let options = self.options.clone();
		let document = Document {
			style: stylesheet(),
			before,
		};
		let handler = leptos_axum::render_app_async_with_context(
			move || {
				provide_context(page.clone());
				provide_context(document.clone());
			},
			move || demo_pages::shell(options.clone()),
		);
		// leptos reads the request for its router and for redirects; a page
		// here is chosen by the route that called this, so it sees none of
		// the visitor's request.
		let request = Request::builder()
			.uri("/")
			.body(Body::empty())
			.unwrap_or_default();

		handler(request).await
	}
}

/// How leptos names the client in a page: `/pkg/demo_pages.js` and its
/// Wasm, from the site's own origin.
fn options() -> LeptosOptions {
	LeptosOptions::builder()
		.output_name(OUTPUT_NAME)
		.site_pkg_dir(PKG_DIR)
		.build()
}

/// A page's whole stylesheet: the faces, the admin's colour tokens, and the
/// demo's own rules.
fn stylesheet() -> String {
	let theme = leptoskit_admin::theme_css();
	let mut style = String::with_capacity(FONT_FACES.len() + theme.len() + STYLE.len());
	style.push_str(FONT_FACES);
	style.push_str(&theme);
	style.push_str(STYLE);
	style
}

/// The media type of a client file, or `None` for a file that is not served
/// by its own name: a compressed copy, or anything a client build does not
/// write.
fn content_type(name: &str) -> Option<&'static str> {
	let extension = Path::new(name).extension()?.to_str()?;

	match extension {
		"js" => Some("text/javascript; charset=utf-8"),
		"wasm" => Some("application/wasm"),
		"json" => Some("application/json"),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn build(dir: &Path, files: &[(&str, &[u8])]) {
		std::fs::create_dir_all(dir.join(PKG_DIR)).expect("a directory");
		for (name, bytes) in files {
			std::fs::write(dir.join(PKG_DIR).join(name), bytes).expect("a file");
		}
	}

	#[test]
	fn a_directory_without_the_client_is_refused() {
		let root = tempfile::tempdir().expect("a directory");
		assert!(Client::load(root.path()).is_err(), "no pkg directory");

		build(root.path(), &[("demo_pages.js", b"export {}")]);
		let error = Client::load(root.path()).expect_err("no wasm");
		assert!(error.to_string().contains("demo_pages_bg.wasm"), "{error}");
	}

	#[test]
	fn the_client_is_served_in_the_smallest_encoding_the_browser_takes() {
		let root = tempfile::tempdir().expect("a directory");
		build(
			root.path(),
			&[
				("demo_pages.js", b"export {}"),
				("demo_pages.js.br", b"br"),
				("demo_pages.js.gz", b"gz"),
				("demo_pages_bg.wasm", b"\0asm"),
				("notes.txt", b"not the client"),
			],
		);
		let client = Client::load(root.path()).expect("the client loads");
		let encoding_for = |accept: Option<&'static str>, name: &str| {
			let mut headers = HeaderMap::new();
			if let Some(accept) = accept {
				headers.insert(header::ACCEPT_ENCODING, HeaderValue::from_static(accept));
			}
			let response = client.serve(name, &headers);
			(
				response.status(),
				response
					.headers()
					.get(header::CONTENT_ENCODING)
					.and_then(|value| value.to_str().ok())
					.map(str::to_owned),
			)
		};

		assert_eq!(
			encoding_for(Some("gzip, br"), "demo_pages.js"),
			(StatusCode::OK, Some("br".to_owned()))
		);
		assert_eq!(
			encoding_for(Some("gzip"), "demo_pages.js"),
			(StatusCode::OK, Some("gzip".to_owned()))
		);
		assert_eq!(
			encoding_for(Some("br;q=0, gzip;q=0"), "demo_pages.js"),
			(StatusCode::OK, None)
		);
		assert_eq!(
			encoding_for(None, "demo_pages_bg.wasm"),
			(StatusCode::OK, None)
		);
		// A compressed copy is an encoding, never a file of its own, and
		// nothing else in the directory is served.
		assert_eq!(
			encoding_for(Some("br"), "demo_pages.js.br").0,
			StatusCode::NOT_FOUND
		);
		assert_eq!(encoding_for(None, "notes.txt").0, StatusCode::NOT_FOUND);
		assert_eq!(encoding_for(None, "../Cargo.toml").0, StatusCode::NOT_FOUND);
	}
}
