//! How a refused request is answered, by the operation routes and by the
//! live transports alike.

use axum::body::Body;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;

use crate::Error;

/// The answer to a failed call: the status, and `Kind|message`. A failure
/// that says how long to wait also says so in `Retry-After`.
pub(crate) fn failure(error: &Error) -> Response {
	let status = StatusCode::from_u16(error.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
	let mut response = Response::new(Body::from(error.to_wire()));
	*response.status_mut() = status;
	response
		.headers_mut()
		.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));

	if let Some(wait) = error.retry_after() {
		response.headers_mut().insert(
			header::RETRY_AFTER,
			HeaderValue::from(crate::operation::whole_seconds(wait)),
		);
	}

	response
}
