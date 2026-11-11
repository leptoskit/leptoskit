//! The service: what `lk` runs when an app declares `[services.analytics]`.
//!
//! Two ways in, one pipeline:
//!
//! - `POST v1/batch` takes the binary batch format ([`crate::wire`]), which
//!   leptoskit's own clients send.
//! - The `capture` operation takes the same events as JSON, for clients in
//!   other languages and for curl.
//!
//! Both validate on the request's task ([`accept`]), queue the rows, and
//! answer once the batch writer has committed them ([`ingest`]).

pub mod accept;
pub mod ingest;
pub mod limits;
pub mod otlp;
pub mod tables;

use std::future::Future;
use std::io::Read as _;
use std::sync::Arc;

use leptoskit_axum::axum;
use leptoskit_axum::axum::Router;
use leptoskit_axum::axum::body::Bytes;
use leptoskit_axum::axum::extract::DefaultBodyLimit;
use leptoskit_axum::axum::extract::State;
use leptoskit_axum::axum::http::HeaderMap;
use leptoskit_axum::axum::http::StatusCode;
use leptoskit_axum::axum::http::header;
use leptoskit_axum::axum::response::IntoResponse;
use leptoskit_axum::axum::response::Response;
use leptoskit_axum::axum::routing::post;
use leptoskit_db::Db;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::Identify;
use leptoskit_rpc::server::Identity;
use leptoskit_rpc::server::IdentityLease;
use serde::Deserialize;
use serde::Serialize;

pub use self::accept::JsonBatch;
pub use self::accept::JsonEvent;
pub use self::accept::Refused;
pub use self::accept::Source;
pub use self::ingest::Ack;
pub use self::ingest::Committed;
pub use self::ingest::Ingest;
pub use self::ingest::IngestConfig;
pub use self::ingest::IngestError;
pub use self::limits::Rate;
pub use self::tables::MIGRATIONS;
#[cfg(feature = "postgres")]
pub use self::tables::POSTGRES_MIGRATIONS;
use crate::SERVICE;
use crate::wire;

type BoxFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The largest request body `v1/batch` reads, compressed.
pub const MAX_BODY_BYTES: usize = 4 << 20;

/// The largest a batch may be once decompressed, so a small compressed body
/// can't expand without limit.
pub const MAX_DECOMPRESSED_BYTES: u64 = 16 << 20;

/// What a batch came to: what's now stored, and which entries were refused
/// and why. A refused entry will never be accepted, so clients drop it
/// rather than retry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaptureResult {
	pub stored: u32,
	/// Entries an earlier attempt already stored.
	pub duplicates: u32,
	/// Entries dropped because their `distinct_id` sent too many too fast.
	/// Like refused ones, they're not worth retrying.
	#[serde(default)]
	pub rate_limited: u32,
	pub refused: Vec<Refused>,
}

/// Records a batch of events sent as JSON, and answers once they're stored.
/// Anyone may call it, as a browser would.
#[rpc::action(service = "analytics")]
pub async fn capture(
	cx: &rpc::ActionCx,
	#[serde(default)] sent_at: Option<String>,
	batch: Vec<JsonEvent>,
) -> rpc::Result<CaptureResult> {
	let Some(ingest) = cx.get::<Ingest>() else {
		return Err(rpc::Error::server("the analytics service isn't ingesting"));
	};
	let received_at = leptoskit_db::now();
	let source = if cx.identity().caller.as_deref() == Some(rpc::server::APP_CALLER) {
		Source::Server
	} else {
		Source::Http
	};
	let accepted = accept::json_batch(JsonBatch { sent_at, batch }, received_at, source);
	let refused = accepted.refused.clone();
	let committed = ingest
		.submit(accepted, received_at)
		.await
		.map_err(|error| rpc::Error::retry(error.to_string()))?;
	Ok(CaptureResult {
		stored: committed.stored,
		duplicates: committed.duplicates,
		rate_limited: committed.rate_limited,
		refused,
	})
}

/// Identifies callers with `inner`, and gives every request the ingest
/// queue, which the `capture` operation reads.
struct WithIngest<I> {
	inner: I,
	ingest: Ingest,
}

impl<I: Identify> Identify for WithIngest<I> {
	fn identify<'a>(
		&'a self,
		request: &'a axum::http::request::Parts,
	) -> BoxFuture<'a, rpc::Result<Identity>> {
		Box::pin(async move {
			let identity = self.inner.identify(request).await?;
			Ok(self.with_ingest(identity))
		})
	}

	fn lease<'a>(&'a self, identity: &'a Identity) -> BoxFuture<'a, rpc::Result<IdentityLease>> {
		self.inner.lease(identity)
	}

	fn identify_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, rpc::Result<Identity>> {
		Box::pin(async move {
			let identity = self.inner.identify_token(token).await?;
			Ok(self.with_ingest(identity))
		})
	}
}

impl<I> WithIngest<I> {
	fn with_ingest(&self, mut identity: Identity) -> Identity {
		identity.context = identity.context.with(self.ingest.clone());
		identity
	}
}

#[derive(Clone)]
struct BatchState {
	ingest: Ingest,
	identify: Arc<dyn Identify>,
	db: Db,
}

/// The service's routes, under `/_leptoskit/services/analytics`, with the
/// batch writer started on `db`. Callers presenting `LEPTOSKIT_SERVICE_TOKEN`
/// are the app.
pub fn routes(db: Db, origin: Option<&str>, config: IngestConfig) -> Router {
	use rpc::server::Anonymous;
	use rpc::server::ServiceCallers;

	routes_with(db, origin, config, ServiceCallers::for_service(Anonymous))
}

/// [`routes`], identifying callers with `callers`.
pub fn routes_with(
	db: Db,
	origin: Option<&str>,
	config: IngestConfig,
	callers: impl Identify,
) -> Router {
	let _ = origin;
	let ingest = Ingest::start(db.clone(), config);
	let with_ingest = Arc::new(WithIngest {
		inner: callers,
		ingest: ingest.clone(),
	});
	let server = rpc::server::Server::new(db.clone())
		.service(SERVICE)
		.identify(Arc::clone(&with_ingest));
	let identify: Arc<dyn Identify> = with_ingest;

	let batch = Router::new()
		.route("/v1/batch", post(receive_batch))
		.route("/otlp/v1/metrics", post(receive_metrics))
		.layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
		.with_state(BatchState {
			ingest,
			identify,
			db,
		});

	Router::new().nest(&rpc::service_prefix(SERVICE), server.router().merge(batch))
}

/// `POST v1/batch`: a binary batch, optionally gzipped.
async fn receive_batch(
	State(state): State<BatchState>,
	request: axum::extract::Request,
) -> Response {
	let received_at = leptoskit_db::now();
	let (parts, body) = request.into_parts();
	let identity = match state.identify.identify(&parts).await {
		Ok(identity) => identity,
		Err(error) => return error_response(StatusCode::UNAUTHORIZED, &error.to_string()),
	};
	let Ok(body) = axum::body::to_bytes(body, MAX_BODY_BYTES).await else {
		return error_response(StatusCode::PAYLOAD_TOO_LARGE, "the batch is too large");
	};
	let bytes = match decompress(&parts.headers, &body) {
		Ok(bytes) => bytes,
		Err((status, message)) => return error_response(status, message),
	};
	let batch = match wire::decode(&bytes, &wire::Limits::default()) {
		Ok(batch) => batch,
		Err(error) => return error_response(StatusCode::BAD_REQUEST, &error.to_string()),
	};

	let source = if identity.caller.as_deref() == Some(rpc::server::APP_CALLER) {
		Source::Server
	} else {
		Source::Client
	};
	let accepted = accept::batch(batch, received_at, source);
	let refused = accepted.refused.clone();
	match state.ingest.submit(accepted, received_at).await {
		Ok(committed) => {
			axum::Json(CaptureResult {
				stored: committed.stored,
				duplicates: committed.duplicates,
				rate_limited: committed.rate_limited,
				refused,
			})
			.into_response()
		}
		Err(error) => {
			let mut response = error_response(StatusCode::SERVICE_UNAVAILABLE, &error.to_string());
			response
				.headers_mut()
				.insert(header::RETRY_AFTER, header::HeaderValue::from_static("1"));
			response
		}
	}
}

/// `POST otlp/v1/metrics`: `leptoskit_apm`'s OTLP/HTTP JSON export, from
/// the app only. Answers as OTLP does: `{}`, or `partialSuccess` with the
/// data points it didn't store and why.
async fn receive_metrics(
	State(state): State<BatchState>,
	request: axum::extract::Request,
) -> Response {
	let (parts, body) = request.into_parts();
	let identity = match state.identify.identify(&parts).await {
		Ok(identity) => identity,
		Err(error) => return error_response(StatusCode::UNAUTHORIZED, &error.to_string()),
	};
	if identity.caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return error_response(
			StatusCode::FORBIDDEN,
			"only the app sends metrics: sign the export with the service's token",
		);
	}
	let Ok(body) = axum::body::to_bytes(body, MAX_BODY_BYTES).await else {
		return error_response(StatusCode::PAYLOAD_TOO_LARGE, "the export is too large");
	};
	let bytes = match decompress(&parts.headers, &body) {
		Ok(bytes) => bytes,
		Err((status, message)) => return error_response(status, message),
	};
	let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
		return error_response(StatusCode::BAD_REQUEST, "the export isn't OTLP JSON");
	};
	let parsed = otlp::parse(&json);
	if let Err(error) = otlp::write(&state.db, &parsed.windows).await {
		let mut response = error_response(StatusCode::SERVICE_UNAVAILABLE, &error.to_string());
		response
			.headers_mut()
			.insert(header::RETRY_AFTER, header::HeaderValue::from_static("1"));
		return response;
	}
	let answer = if parsed.rejected == 0 {
		serde_json::json!({})
	} else {
		serde_json::json!({ "partialSuccess": {
			"rejectedDataPoints": parsed.rejected.to_string(),
			"errorMessage": parsed.reason.unwrap_or_default(),
		}})
	};
	axum::Json(answer).into_response()
}

/// The body, gunzipped when `Content-Encoding` says so, within
/// [`MAX_DECOMPRESSED_BYTES`].
fn decompress(headers: &HeaderMap, body: &Bytes) -> Result<Vec<u8>, (StatusCode, &'static str)> {
	let encoding = headers
		.get(header::CONTENT_ENCODING)
		.and_then(|value| value.to_str().ok())
		.unwrap_or("identity")
		.trim()
		.to_ascii_lowercase();
	match encoding.as_str() {
		"identity" | "" => Ok(body.to_vec()),
		"gzip" => {
			let mut out = Vec::new();
			let read = flate2::read::GzDecoder::new(&body[..])
				.take(MAX_DECOMPRESSED_BYTES + 1)
				.read_to_end(&mut out);
			match read {
				Ok(_) if out.len() as u64 > MAX_DECOMPRESSED_BYTES => {
					Err((
						StatusCode::PAYLOAD_TOO_LARGE,
						"the batch is too large once decompressed",
					))
				}
				Ok(_) => Ok(out),
				Err(_) => Err((StatusCode::BAD_REQUEST, "the body isn't valid gzip")),
			}
		}
		_ => {
			Err((
				StatusCode::UNSUPPORTED_MEDIA_TYPE,
				"only gzip and identity encodings are accepted",
			))
		}
	}
}

fn error_response(status: StatusCode, message: &str) -> Response {
	(status, axum::Json(serde_json::json!({ "error": message }))).into_response()
}
