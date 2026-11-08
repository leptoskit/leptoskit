//! Browser Direct camera: `getUserMedia` sessions attached to
//! `Camera` views that render `<video data-lk-camera="id">`. Torch is
//! `Unavailable`. Still capture draws the preview onto a canvas and returns a
//! `blob:` URI when under the size budget.

use std::cell::RefCell;
use std::collections::HashMap;

use js_sys::Array;
use js_sys::Reflect;
use js_sys::Uint8Array;
use leptoskit_bridge::Error;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::HtmlCanvasElement;
use web_sys::HtmlVideoElement;
use web_sys::MediaStream;
use web_sys::MediaStreamConstraints;

use crate::Asset;
use crate::CREATE;
use crate::Create;
use crate::Facing;
use crate::RELEASE;
use crate::SET_TORCH;
use crate::STATUS;
use crate::SessionCommand;
use crate::SessionId;
use crate::Status;
use crate::TAKE_PICTURE;
use crate::TakePicture;
use crate::Torch;

thread_local! {
	static SESSIONS: RefCell<HashMap<u32, Session>> = RefCell::new(HashMap::new());
	static NEXT_ID: RefCell<u32> = const { RefCell::new(1) };
}

struct Session {
	facing: Facing,
	stream: MediaStream,
	element: HtmlVideoElement,
}

/// Direct browser camera API mirroring the native guest methods.
#[derive(Debug, Clone, Copy, Default)]
pub struct Camera;

impl Camera {
	/// Opens a `getUserMedia` session for `facing`.
	pub async fn create(self, facing: Facing) -> Result<SessionId, Error> {
		let stream = request_stream(facing).await?;
		let element = document_video()?;
		element.set_src_object(Some(&stream));
		element.set_muted(true);
		let _ = element.set_attribute("playsinline", "");
		let _ = element.play();
		let id = NEXT_ID.with(|next| {
			let id = *next.borrow();
			*next.borrow_mut() = id.saturating_add(1).max(1);
			id
		});
		SESSIONS.with(|sessions| {
			sessions.borrow_mut().insert(
				id,
				Session {
					facing,
					stream,
					element,
				},
			);
		});
		attach_surfaces();
		Ok(SessionId(id))
	}

	/// Torch is not exposed on the portable browser surface.
	pub fn set_torch(self, session: SessionId, _enabled: bool) -> Result<(), Error> {
		with_session(session, |_| Err(Error::Unavailable))
	}

	/// Captures the current preview frame into a `blob:` JPEG under
	/// `max_bytes`.
	pub async fn take_picture(self, request: TakePicture) -> Result<Asset, Error> {
		let (width, height, data_url) = with_session(request.session, |session| {
			let width = session.element.video_width();
			let height = session.element.video_height();
			if width == 0 || height == 0 {
				return Err(Error::Unavailable);
			}
			let canvas = document_canvas()?;
			canvas.set_width(width);
			canvas.set_height(height);
			let context = canvas
				.get_context("2d")
				.map_err(|_| Error::Native)?
				.ok_or(Error::Native)?
				.dyn_into::<web_sys::CanvasRenderingContext2d>()
				.map_err(|_| Error::Native)?;
			context
				.draw_image_with_html_video_element(&session.element, 0.0, 0.0)
				.map_err(|_| Error::Native)?;
			let data_url = canvas
				.to_data_url_with_type("image/jpeg")
				.map_err(|_| Error::Native)?;
			Ok((width, height, data_url))
		})?;
		let bytes = data_url_to_bytes(&data_url)?;
		if bytes.len() > request.max_bytes as usize {
			return Err(Error::TooLarge);
		}
		let blob_parts = Array::new();
		blob_parts.push(&Uint8Array::from(bytes.as_slice()));
		let properties = web_sys::BlobPropertyBag::new();
		properties.set_type("image/jpeg");
		let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&blob_parts, &properties)
			.map_err(|_| Error::Native)?;
		let uri = web_sys::Url::create_object_url_with_blob(&blob).map_err(|_| Error::Native)?;
		Ok(Asset {
			uri,
			mime_type: Some("image/jpeg".into()),
			width: Some(width),
			height: Some(height),
			size_bytes: Some(bytes.len() as u64),
		})
	}

	/// Snapshots facing and active state; torch is never available here.
	pub fn status(self, session: SessionId) -> Result<Status, Error> {
		with_session(session, |session| {
			Ok(Status {
				facing: session.facing,
				torch_on: false,
				torch_available: false,
				active: session.stream.get_tracks().length() > 0,
			})
		})
	}

	/// Stops tracks and drops the session.
	pub fn release(self, session: SessionId) -> Result<(), Error> {
		let removed = SESSIONS.with(|sessions| sessions.borrow_mut().remove(&session.0));
		let Some(session) = removed else {
			return Err(Error::InvalidArgument);
		};
		for track in session.stream.get_tracks() {
			if let Ok(track) = track.dyn_into::<web_sys::MediaStreamTrack>() {
				track.stop();
			}
		}
		session.element.set_src_object(None);
		attach_surfaces();
		Ok(())
	}

	/// Dispatches a Direct method for hosts that speak the wire protocol.
	pub async fn invoke(
		self,
		method: u32,
		payload: &[u8],
	) -> Result<leptoskit_bridge::Bytes, Error> {
		match method {
			x if x == CREATE.0 => {
				let create = Create::decode(payload)?;
				Ok(self.create(create.facing).await?.encode())
			}
			x if x == SET_TORCH.0 => {
				let torch = Torch::decode(payload)?;
				self.set_torch(torch.session, torch.enabled)?;
				Ok(empty_bytes())
			}
			x if x == TAKE_PICTURE.0 => {
				let request = TakePicture::decode(payload)?;
				Ok(self.take_picture(request).await?.encode()?)
			}
			x if x == STATUS.0 => {
				let session = SessionCommand::decode(payload)?.session;
				Ok(self.status(session)?.encode())
			}
			x if x == RELEASE.0 => {
				self.release(SessionCommand::decode(payload)?.session)?;
				Ok(empty_bytes())
			}
			_ => Err(Error::Unavailable),
		}
	}
}

fn empty_bytes() -> leptoskit_bridge::Bytes {
	leptoskit_bridge::codec::Encoder::with_capacity(0).finish()
}

fn with_session<T>(
	session: SessionId,
	body: impl FnOnce(&mut Session) -> Result<T, Error>,
) -> Result<T, Error> {
	SESSIONS.with(|sessions| {
		let mut sessions = sessions.borrow_mut();
		let session = sessions.get_mut(&session.0).ok_or(Error::InvalidArgument)?;
		body(session)
	})
}

async fn request_stream(facing: Facing) -> Result<MediaStream, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	let navigator = window.navigator();
	let devices = navigator.media_devices().map_err(|_| Error::Unavailable)?;
	let constraints = MediaStreamConstraints::new();
	let video = js_sys::Object::new();
	let facing_mode = match facing {
		Facing::Back => "environment",
		Facing::Front => "user",
	};
	Reflect::set(
		&video,
		&JsValue::from_str("facingMode"),
		&JsValue::from_str(facing_mode),
	)
	.map_err(|_| Error::Native)?;
	constraints.set_video(&video);
	constraints.set_audio(&JsValue::from_bool(false));
	let promise = devices
		.get_user_media_with_constraints(&constraints)
		.map_err(map_dom_error)?;
	let value = JsFuture::from(promise).await.map_err(map_dom_error)?;
	value.dyn_into::<MediaStream>().map_err(|_| Error::Native)
}

fn map_dom_error(error: JsValue) -> Error {
	let name = Reflect::get(&error, &JsValue::from_str("name"))
		.ok()
		.and_then(|value| value.as_string())
		.unwrap_or_default();
	match name.as_str() {
		"NotAllowedError" | "SecurityError" => Error::PermissionDenied,
		"NotFoundError" | "OverconstrainedError" | "NotReadableError" => Error::Unavailable,
		_ => Error::Native,
	}
}

fn document_video() -> Result<HtmlVideoElement, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	let document = window.document().ok_or(Error::Unavailable)?;
	let element = document
		.create_element("video")
		.map_err(|_| Error::Native)?;
	element
		.dyn_into::<HtmlVideoElement>()
		.map_err(|_| Error::Native)
}

fn document_canvas() -> Result<HtmlCanvasElement, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	let document = window.document().ok_or(Error::Unavailable)?;
	let element = document
		.create_element("canvas")
		.map_err(|_| Error::Native)?;
	element
		.dyn_into::<HtmlCanvasElement>()
		.map_err(|_| Error::Native)
}

fn data_url_to_bytes(data_url: &str) -> Result<Vec<u8>, Error> {
	let Some((_, b64)) = data_url.split_once(',') else {
		return Err(Error::Native);
	};
	base64_decode(b64).map_err(|_| Error::Native)
}

fn base64_decode(input: &str) -> Result<Vec<u8>, ()> {
	fn value(byte: u8) -> Result<u8, ()> {
		match byte {
			b'A'..=b'Z' => Ok(byte - b'A'),
			b'a'..=b'z' => Ok(byte - b'a' + 26),
			b'0'..=b'9' => Ok(byte - b'0' + 52),
			b'+' => Ok(62),
			b'/' => Ok(63),
			_ => Err(()),
		}
	}
	let cleaned: Vec<u8> = input
		.bytes()
		.filter(|byte| !byte.is_ascii_whitespace())
		.collect();
	if cleaned.len() % 4 != 0 {
		return Err(());
	}
	let mut output = Vec::with_capacity(cleaned.len() / 4 * 3);
	for chunk in cleaned.chunks(4) {
		let a = value(chunk[0])?;
		let b = value(chunk[1])?;
		let pad2 = chunk[2] == b'=';
		let pad3 = chunk[3] == b'=';
		let c = if pad2 { 0 } else { value(chunk[2])? };
		let d = if pad3 { 0 } else { value(chunk[3])? };
		output.push((a << 2) | (b >> 4));
		if !pad2 {
			output.push((b << 4) | (c >> 2));
		}
		if !pad3 {
			output.push((c << 6) | d);
		}
	}
	Ok(output)
}

/// Copies each session's MediaStream onto matching `data-lk-camera` nodes.
fn attach_surfaces() {
	let Some(window) = web_sys::window() else {
		return;
	};
	let Some(document) = window.document() else {
		return;
	};
	SESSIONS.with(|sessions| {
		let sessions = sessions.borrow();
		for (id, session) in sessions.iter() {
			let selector = format!("video[data-lk-camera=\"{id}\"]");
			let Ok(nodes) = document.query_selector_all(&selector) else {
				continue;
			};
			for index in 0..nodes.length() {
				let Some(node) = nodes.item(index) else {
					continue;
				};
				let Ok(surface) = node.dyn_into::<HtmlVideoElement>() else {
					continue;
				};
				surface.set_src_object(Some(&session.stream));
				surface.set_muted(true);
				let _ = surface.set_attribute("playsinline", "");
				let _ = surface.play();
			}
		}
	});
}

#[cfg(test)]
mod tests {
	use wasm_bindgen_test::wasm_bindgen_test;

	use super::*;

	wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

	#[test]
	fn base64_decode_round_trips_and_rejects_junk() {
		assert_eq!(base64_decode("AQID").unwrap(), vec![1, 2, 3]);
		assert_eq!(base64_decode("AQ==").unwrap(), vec![1]);
		assert_eq!(base64_decode("AQI=").unwrap(), vec![1, 2]);
		assert!(base64_decode("A").is_err());
		assert!(base64_decode("!!!!").is_err());
		assert_eq!(data_url_to_bytes("not-a-data-url"), Err(Error::Native));
		assert_eq!(
			data_url_to_bytes("data:image/jpeg;base64,AQID").unwrap(),
			vec![1, 2, 3]
		);
		assert_eq!(crate::MAX_BYTES, 16 * 1024 * 1024);
	}

	#[wasm_bindgen_test]
	fn map_dom_error_classifies_permission_and_hardware() {
		fn err(name: &str) -> Error {
			let object = js_sys::Object::new();
			Reflect::set(
				&object,
				&JsValue::from_str("name"),
				&JsValue::from_str(name),
			)
			.unwrap();
			map_dom_error(object.into())
		}
		assert_eq!(err("NotAllowedError"), Error::PermissionDenied);
		assert_eq!(err("SecurityError"), Error::PermissionDenied);
		assert_eq!(err("NotFoundError"), Error::Unavailable);
		assert_eq!(err("OverconstrainedError"), Error::Unavailable);
		assert_eq!(err("NotReadableError"), Error::Unavailable);
		assert_eq!(err("TypeError"), Error::Native);
	}
}
