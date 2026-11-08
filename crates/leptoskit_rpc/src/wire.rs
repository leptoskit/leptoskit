//! Request and response bodies on the HTTP operation routes: LK Pod and JSON.

use serde_json::Value;

pub(crate) use crate::JSON_CONTENT_TYPE;
pub(crate) use crate::POD_CONTENT_TYPE;

/// Which wire encoding a call uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
	/// `application/vnd.leptoskit.rpc.v1+bin`.
	Pod,
	/// `application/json`.
	Json,
}

/// Which transports the server allows on operation routes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportSet {
	/// LK Pod.
	pub pod: bool,
	/// JSON through serde.
	pub json: bool,
}

impl Default for TransportSet {
	/// Pod and JSON, so existing JSON callers keep working until apps tighten
	/// config; product default in `leptoskit.toml` is `["pod"]` only.
	fn default() -> Self {
		Self {
			pod: true,
			json: true,
		}
	}
}

impl TransportSet {
	/// Builds the set from `[rpc].transports` names (`pod`, `json`).
	#[must_use]
	pub fn from_names(names: &[String]) -> Self {
		let mut set = Self {
			pod: false,
			json: false,
		};

		for name in names {
			match name.as_str() {
				"pod" => set.pod = true,
				"json" => set.json = true,
				_ => {}
			}
		}

		if !set.pod && !set.json {
			set.pod = true;
		}

		set
	}

	/// Whether `transport` is allowed.
	#[must_use]
	pub const fn allows(self, transport: Transport) -> bool {
		match transport {
			Transport::Pod => self.pod,
			Transport::Json => self.json,
		}
	}
}

/// Arguments on the wire before they reach a handler.
#[derive(Clone, Debug)]
pub enum WireArgs {
	/// One JSON object, as today.
	Json(Value),
	/// LK Pod bytes for the operation's argument struct.
	Pod(Vec<u8>),
}

impl WireArgs {
	/// Empty arguments: `{}` for JSON, a version-0 struct header with no fields
	/// for Pod.
	#[must_use]
	pub fn empty(transport: Transport) -> Self {
		match transport {
			Transport::Json => Self::Json(Value::Object(serde_json::Map::new())),
			Transport::Pod => Self::Pod(empty_pod_struct()),
		}
	}
}

/// A successful result on the wire.
#[derive(Clone, Debug)]
pub enum WireBody {
	/// JSON value.
	Json(Value),
	/// LK Pod bytes for the operation's output type.
	Pod(Vec<u8>),
}

fn empty_pod_struct() -> Vec<u8> {
	let mut writer = leptoskit_pod::Writer::new();
	let _ = leptoskit_pod::encode_struct_v0(&mut writer, |_| Ok(()));
	writer.finish()
}

/// Parses `Content-Type` into a [`Transport`] the server allows.
pub(crate) fn request_transport(
	content_type: Option<&str>,
	allowed: TransportSet,
) -> Result<Transport, &'static str> {
	let requested = content_type.map(media_type).unwrap_or(POD_CONTENT_TYPE);

	if requested == POD_CONTENT_TYPE {
		if allowed.pod {
			return Ok(Transport::Pod);
		}
		return Err("this server does not accept LK Pod on this route");
	}

	if requested == JSON_CONTENT_TYPE {
		if allowed.json {
			return Ok(Transport::Json);
		}
		return Err("this server does not accept JSON on this route");
	}

	if allowed.pod {
		Ok(Transport::Pod)
	} else if allowed.json {
		Ok(Transport::Json)
	} else {
		Err("no operation transport is enabled")
	}
}

/// Picks the response encoding from `Accept` and what the server allows.
#[must_use]
pub(crate) fn response_transport(
	accept: Option<&str>,
	allowed: TransportSet,
	request: Transport,
) -> Transport {
	let Some(accept) = accept.map(str::trim) else {
		return request;
	};

	if accept.is_empty() || accept == "*/*" {
		if allowed.pod {
			return Transport::Pod;
		}
		if allowed.json {
			return Transport::Json;
		}
		return request;
	}

	for part in accept.split(',') {
		if media_type(part.trim()) == POD_CONTENT_TYPE && allowed.pod {
			return Transport::Pod;
		}
	}

	for part in accept.split(',') {
		if media_type(part.trim()) == JSON_CONTENT_TYPE && allowed.json {
			return Transport::Json;
		}
	}

	if allowed.pod {
		Transport::Pod
	} else if allowed.json {
		Transport::Json
	} else {
		request
	}
}

fn media_type(value: &str) -> &str {
	value.split(';').next().unwrap_or(value).trim()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn config_transports_name_pod_and_json() {
		let both = TransportSet::from_names(&["pod".into(), "json".into()]);
		assert!(both.allows(Transport::Pod));
		assert!(both.allows(Transport::Json));

		let pod = TransportSet::from_names(&["pod".into()]);
		assert!(pod.allows(Transport::Pod));
		assert!(!pod.allows(Transport::Json));

		let unknown = TransportSet::from_names(&["ftp".into()]);
		assert!(unknown.allows(Transport::Pod));
	}

	#[test]
	fn negotiate_request_and_response() {
		let both = TransportSet::default();
		assert_eq!(
			request_transport(Some(POD_CONTENT_TYPE), both).unwrap(),
			Transport::Pod
		);
		assert_eq!(
			request_transport(Some(JSON_CONTENT_TYPE), both).unwrap(),
			Transport::Json
		);
		assert_eq!(
			response_transport(
				Some("application/json, application/vnd.leptoskit.rpc.v1+bin"),
				both,
				Transport::Json
			),
			Transport::Pod
		);
		assert_eq!(
			response_transport(None, both, Transport::Json),
			Transport::Json
		);
	}
}
