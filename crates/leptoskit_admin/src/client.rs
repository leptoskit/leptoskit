//! The pages' browser half: the Wasm client `leptoskit build` made from
//! `leptoskit_admin_pages`, and the document a page is rendered into.
//!
//! The client is read once, when the admin is built, and served from memory
//! from then on, so the markup the admin renders and the client it hands
//! out always come from the same start, which is what hydration needs.
//!
//! The directory is a build's `--out` directory: the client sits in its
//! `pkg/`, and its files are named after the client library,
//! `leptoskit_admin_pages`. Without one, the admin's pages render and stay
//! as the server drew them.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse as _;
use axum::response::Response;
use leptos::config::LeptosOptions;
use leptos::prelude::provide_context;
use leptoskit_admin_pages::Model;

use crate::AdminError;

/// The client library's name, which names its files.
pub(crate) const OUTPUT_NAME: &str = "leptoskit_admin_pages";

/// The directory under the site root that holds the client.
pub(crate) const PKG_DIR: &str = "pkg";

/// The most one client file may weigh. The whole client is a few hundred
/// kilobytes; anything near this is not the client.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// The most files the client directory may hold.
const MAX_FILES: usize = 64;

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
pub(crate) struct Client {
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
	pub(crate) fn absent() -> Self {
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
	/// client: no `leptoskit_admin_pages.js` or
	/// `leptoskit_admin_pages_bg.wasm`, a file that is not a regular file,
	/// or more or larger files than a client has.
	pub(crate) fn load(site_root: &Path) -> Result<Self, AdminError> {
		let dir = site_root.join(PKG_DIR);
		let entries = std::fs::read_dir(&dir).map_err(|error| {
			AdminError::Config(format!(
				"the pages' client is not at {}: build it with `leptoskit build --release \
				 --platform web` in crates/leptoskit_admin_pages and point `client_dir` at its \
				 output ({error})",
				dir.display()
			))
		})?;
		let mut raw = BTreeMap::<String, axum::body::Bytes>::new();

		for entry in entries {
			let entry =
				entry.map_err(|error| failed(format!("listing {}: {error}", dir.display())))?;
			let path = entry.path();
			let metadata = std::fs::symlink_metadata(&path)
				.map_err(|error| failed(format!("inspecting {}: {error}", path.display())))?;
			ensure(metadata.is_file(), || {
				format!(
					"{} is not a regular file; the client directory holds only the built client",
					path.display()
				)
			})?;
			ensure(metadata.len() <= MAX_FILE_BYTES, || {
				format!("{} is larger than a client file can be", path.display())
			})?;
			let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
				return Err(failed(format!(
					"{} has a name that is not text",
					path.display()
				)));
			};
			ensure(raw.len() < MAX_FILES, || {
				format!("{} holds more files than a client has", dir.display())
			})?;
			let bytes = std::fs::read(&path)
				.map_err(|error| failed(format!("reading {}: {error}", path.display())))?;
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
			ensure(
				files
					.get(&required)
					.is_some_and(|file| !file.plain.is_empty()),
				|| {
					format!(
						"{} has no {required}; it is not the pages' built client",
						dir.display()
					)
				},
			)?;
		}

		Ok(Self {
			options: options(),
			files: Arc::new(files),
		})
	}

	/// `GET /pkg/{file}`: one file of the client, in the smallest encoding
	/// the browser accepts. The client's names carry no hash, so a browser
	/// asks again each time and a release is never served stale.
	pub(crate) fn serve(&self, name: &str, headers: &HeaderMap) -> Response {
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

	/// Renders `model`'s page as a whole document: the markup the browser
	/// shows at once, the model the client reads back, and the loader that
	/// fetches the client.
	pub(crate) async fn render(&self, model: Model) -> Response {
		// The pages spawn their work on leptos's executor, which is tokio
		// here. Setting it twice is refused, and that is fine.
		let _ = any_spawner::Executor::init_tokio();
		let options = self.options.clone();
		let handler = leptos_axum::render_app_async_with_context(
			move || provide_context(model.clone()),
			move || leptoskit_admin_pages::document(options.clone()),
		);
		// leptos reads the request for its router and for redirects; a page
		// here is chosen by the route that called this, so it sees none of
		// the admin's request.
		let request = Request::builder()
			.uri("/")
			.body(Body::empty())
			.unwrap_or_default();

		crate::server::no_store(handler(request).await)
	}
}

/// How leptos names the client in a page: `{BASE}/pkg/leptoskit_admin_pages.js`
/// and its Wasm, from the admin's own origin.
fn options() -> LeptosOptions {
	LeptosOptions::builder()
		.output_name(OUTPUT_NAME)
		.site_pkg_dir(format!("{}/{PKG_DIR}", crate::BASE.trim_start_matches('/')))
		.build()
}

fn failed(message: String) -> AdminError {
	AdminError::Config(message)
}

/// `Err` with `message` unless `holds`.
fn ensure(holds: bool, message: impl FnOnce() -> String) -> Result<(), AdminError> {
	if holds {
		Ok(())
	} else {
		Err(failed(message()))
	}
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

		build(root.path(), &[("leptoskit_admin_pages.js", b"export {}")]);
		let error = Client::load(root.path()).expect_err("no wasm");
		assert!(
			error.to_string().contains("leptoskit_admin_pages_bg.wasm"),
			"{error}"
		);
	}

	#[test]
	fn the_client_is_served_in_the_smallest_encoding_the_browser_takes() {
		let root = tempfile::tempdir().expect("a directory");
		build(
			root.path(),
			&[
				("leptoskit_admin_pages.js", b"export {}"),
				("leptoskit_admin_pages.js.br", b"br"),
				("leptoskit_admin_pages.js.gz", b"gz"),
				("leptoskit_admin_pages_bg.wasm", b"\0asm"),
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
			encoding_for(Some("gzip, br"), "leptoskit_admin_pages.js"),
			(StatusCode::OK, Some("br".to_owned()))
		);
		assert_eq!(
			encoding_for(Some("gzip"), "leptoskit_admin_pages.js"),
			(StatusCode::OK, Some("gzip".to_owned()))
		);
		assert_eq!(
			encoding_for(Some("br;q=0, gzip;q=0"), "leptoskit_admin_pages.js"),
			(StatusCode::OK, None)
		);
		assert_eq!(
			encoding_for(None, "leptoskit_admin_pages_bg.wasm"),
			(StatusCode::OK, None)
		);
		// A compressed copy is an encoding, never a file of its own, and
		// nothing else in the directory is served.
		assert_eq!(
			encoding_for(Some("br"), "leptoskit_admin_pages.js.br").0,
			StatusCode::NOT_FOUND
		);
		assert_eq!(encoding_for(None, "notes.txt").0, StatusCode::NOT_FOUND);
		assert_eq!(encoding_for(None, "../Cargo.toml").0, StatusCode::NOT_FOUND);
	}
}
