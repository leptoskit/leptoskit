//! Request and response bodies on the HTTP operation routes.
//!
//! The server speaks JSON through serde. A client that sends the retired LK
//! Pod content type is refused so the mismatch is obvious.

use serde_json::Value;

pub(crate) use crate::JSON_CONTENT_TYPE;

/// Content type of the retired LK Pod encoding. It is not a configured
/// transport; requests that send it are rejected.
const RETIRED_POD_CONTENT_TYPE: &str = "application/vnd.leptoskit.rpc.v1+bin";

/// Which wire encoding a call uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
	/// `application/json`.
	Json,
}

/// Which transports the server allows on operation routes.
///
/// JSON is the only transport. The flag stays so `[rpc].transports` can turn
/// the routes off explicitly, and so another encoding can be added later
/// without changing the call shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportSet {
	/// JSON through serde.
	pub json: bool,
}

impl Default for TransportSet {
	fn default() -> Self {
		Self { json: true }
	}
}

impl TransportSet {
	/// Builds the set from `[rpc].transports` names. The only name is `json`.
	/// An empty list is JSON, matching the configuration default.
	#[must_use]
	pub fn from_names(names: &[String]) -> Self {
		if names.is_empty() {
			return Self { json: true };
		}

		Self {
			json: names.iter().any(|name| name == "json"),
		}
	}

	/// Whether `transport` is allowed.
	#[must_use]
	pub const fn allows(self, transport: Transport) -> bool {
		match transport {
			Transport::Json => self.json,
		}
	}
}

/// Arguments on the wire before they reach a handler.
#[derive(Clone, Debug)]
pub enum WireArgs {
	/// One JSON object.
	Json(Value),
}

impl WireArgs {
	/// Empty arguments: `{}`.
	#[must_use]
	pub fn empty(transport: Transport) -> Self {
		match transport {
			Transport::Json => Self::Json(Value::Object(serde_json::Map::new())),
		}
	}
}

/// A successful result on the wire.
#[derive(Clone, Debug)]
pub enum WireBody {
	/// JSON value.
	Json(Value),
}

/// Parses `Content-Type` into a [`Transport`] the server allows.
///
/// A missing content type is JSON. The retired Pod content type is rejected.
pub(crate) fn request_transport(
	content_type: Option<&str>,
	allowed: TransportSet,
) -> Result<Transport, &'static str> {
	let requested = content_type.map_or(JSON_CONTENT_TYPE, media_type);

	if requested == RETIRED_POD_CONTENT_TYPE {
		return Err("this server speaks JSON; send application/json");
	}

	if !allowed.json {
		return Err("JSON is not enabled for this server");
	}

	if requested == JSON_CONTENT_TYPE {
		return Ok(Transport::Json);
	}

	Err("send the arguments as application/json")
}

/// Responses are JSON. `Accept` is ignored until another transport exists.
#[must_use]
pub(crate) fn response_transport(
	accept: Option<&str>,
	allowed: TransportSet,
	request: Transport,
) -> Transport {
	let _ = (accept, allowed, request);
	Transport::Json
}

fn media_type(value: &str) -> &str {
	value.split(';').next().unwrap_or(value).trim()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn config_transports_name_json() {
		let json = TransportSet::from_names(&["json".into()]);
		assert!(json.allows(Transport::Json));

		let empty = TransportSet::from_names(&[]);
		assert!(empty.allows(Transport::Json));

		let unknown = TransportSet::from_names(&["pod".into()]);
		assert!(!unknown.allows(Transport::Json));
	}

	#[test]
	fn negotiate_request_and_response() {
		let json = TransportSet::default();
		assert!(request_transport(Some(RETIRED_POD_CONTENT_TYPE), json).is_err());
		assert_eq!(
			request_transport(Some(JSON_CONTENT_TYPE), json).unwrap(),
			Transport::Json
		);
		assert_eq!(request_transport(None, json).unwrap(), Transport::Json);
		assert!(request_transport(Some(JSON_CONTENT_TYPE), TransportSet { json: false }).is_err());
		assert_eq!(
			response_transport(Some("application/json"), json, Transport::Json),
			Transport::Json
		);
	}
}
