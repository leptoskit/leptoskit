//! Browser library picker. Call/poll directly inside a trusted click handler.
//!
//! Uses a hidden file input with `showPicker` and the `cancel` event. The reply
//! URI is a `blob:` object URL the page owns; revoke it when finished. Dropping
//! the future removes listeners and the input but cannot dismiss an
//! already-open system picker.
use std::cell::RefCell;
use std::rc::Rc;

use futures::channel::oneshot;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

use crate::Asset;
use crate::Error;
use crate::MAX_MIME_BYTES;
use crate::MAX_URI_BYTES;
use crate::MediaKind;
use crate::Options;

struct InputGuard {
	input: web_sys::HtmlInputElement,
	change: Closure<dyn FnMut(web_sys::Event)>,
	cancel: Closure<dyn FnMut(web_sys::Event)>,
}

impl Drop for InputGuard {
	fn drop(&mut self) {
		self.input.set_onchange(None);
		let _ = self
			.input
			.remove_event_listener_with_callback("cancel", self.cancel.as_ref().unchecked_ref());
		self.input.remove();
	}
}

/// Presents the browser's file input for library media.
pub async fn pick(options: Options) -> Result<Asset, Error> {
	let document = web_sys::window()
		.and_then(|window| window.document())
		.ok_or(Error::Unavailable)?;
	let input = document
		.create_element("input")
		.map_err(|_| Error::Native)?
		.dyn_into::<web_sys::HtmlInputElement>()
		.map_err(|_| Error::Native)?;
	input.set_type("file");
	input.set_hidden(true);
	input.set_accept(accept(options.media()));

	if !js_sys::Reflect::has(input.as_ref(), &"oncancel".into()).map_err(|_| Error::Native)?
		|| !js_sys::Reflect::get(input.as_ref(), &"showPicker".into())
			.map_err(|_| Error::Native)?
			.is_function()
	{
		return Err(Error::Unavailable);
	}

	let (sender, receiver) = oneshot::channel();
	let sender = Rc::new(RefCell::new(Some(sender)));
	let changed = sender.clone();
	let selected = input.clone();
	let change = Closure::wrap(Box::new(move |_: web_sys::Event| {
		if let Some(sender) = changed.borrow_mut().take() {
			let result = selected
				.files()
				.and_then(|files| files.get(0))
				.ok_or(Error::Cancelled);
			let _ = sender.send(result);
		}
	}) as Box<dyn FnMut(web_sys::Event)>);
	let cancel = Closure::wrap(Box::new(move |_: web_sys::Event| {
		if let Some(sender) = sender.borrow_mut().take() {
			let _ = sender.send(Err(Error::Cancelled));
		}
	}) as Box<dyn FnMut(web_sys::Event)>);
	let guard = InputGuard {
		input,
		change,
		cancel,
	};
	guard
		.input
		.set_onchange(Some(guard.change.as_ref().unchecked_ref()));
	guard
		.input
		.add_event_listener_with_callback("cancel", guard.cancel.as_ref().unchecked_ref())
		.map_err(|_| Error::Native)?;
	document
		.body()
		.ok_or(Error::Unavailable)?
		.append_child(&guard.input)
		.map_err(|_| Error::Native)?;
	guard.input.show_picker().map_err(|error| failure(&error))?;
	let file = receiver.await.map_err(|_| Error::Cancelled)??;
	from_file(&file, options).await
}

/// Builds an [`Asset`] from an already selected browser File.
pub async fn from_file(file: &web_sys::File, options: Options) -> Result<Asset, Error> {
	let size = file.size();
	if !size.is_finite() || size < 0.0 {
		return Err(Error::Native);
	}
	if size > f64::from(options.max_bytes()) {
		return Err(Error::TooLarge);
	}
	let mime = file.type_();
	if mime.len() > MAX_MIME_BYTES {
		return Err(Error::TooLarge);
	}
	let uri = web_sys::Url::create_object_url_with_blob(file).map_err(|_| Error::Native)?;
	if uri.is_empty() || uri.len() > MAX_URI_BYTES {
		let _ = web_sys::Url::revoke_object_url(&uri);
		return Err(Error::TooLarge);
	}
	let (width, height) = image_size(&uri, &mime).await;
	Ok(Asset {
		uri,
		mime_type: (!mime.is_empty()).then_some(mime),
		width,
		height,
		size_bytes: Some(size as u64),
	})
}

fn accept(media: MediaKind) -> &'static str {
	match media {
		MediaKind::Images => "image/*",
		MediaKind::Videos => "video/*",
		MediaKind::All => "image/*,video/*",
	}
}

async fn image_size(uri: &str, mime: &str) -> (Option<u32>, Option<u32>) {
	if !mime.starts_with("image/") {
		return (None, None);
	}
	let document = match web_sys::window().and_then(|window| window.document()) {
		Some(document) => document,
		None => return (None, None),
	};
	let image = match document.create_element("img") {
		Ok(element) => {
			match element.dyn_into::<web_sys::HtmlImageElement>() {
				Ok(image) => image,
				Err(_) => return (None, None),
			}
		}
		Err(_) => return (None, None),
	};
	let (sender, receiver) = oneshot::channel::<Result<(u32, u32), ()>>();
	let sender = Rc::new(RefCell::new(Some(sender)));
	let loaded = sender.clone();
	let sized = image.clone();
	let load = Closure::wrap(Box::new(move |_: web_sys::Event| {
		if let Some(sender) = loaded.borrow_mut().take() {
			let _ = sender.send(Ok((sized.natural_width(), sized.natural_height())));
		}
	}) as Box<dyn FnMut(web_sys::Event)>);
	let error = Closure::wrap(Box::new(move |_: web_sys::Event| {
		if let Some(sender) = sender.borrow_mut().take() {
			let _ = sender.send(Err(()));
		}
	}) as Box<dyn FnMut(web_sys::Event)>);
	image.set_onload(Some(load.as_ref().unchecked_ref()));
	image.set_onerror(Some(error.as_ref().unchecked_ref()));
	image.set_src(uri);
	let _keep = (load, error);
	match receiver.await {
		Ok(Ok((width, height))) if width > 0 && height > 0 => (Some(width), Some(height)),
		_ => (None, None),
	}
}

fn failure(error: &JsValue) -> Error {
	match error
		.dyn_ref::<web_sys::DomException>()
		.map(web_sys::DomException::name)
		.as_deref()
	{
		Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied,
		Some("AbortError") => Error::Cancelled,
		Some("NotSupportedError") => Error::Unavailable,
		_ => Error::Native,
	}
}

#[cfg(test)]
mod tests {
	use wasm_bindgen_test::wasm_bindgen_test;

	use super::*;
	wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

	#[wasm_bindgen_test]
	async fn synthetic_image_file_is_bounded() {
		let parts = js_sys::Array::new();
		parts.push(&js_sys::Uint8Array::from(&[0xff_u8, 0xd8, 0xff][..]));
		let file = web_sys::File::new_with_u8_array_sequence_and_options(
			&parts,
			"fixture.jpg",
			web_sys::FilePropertyBag::new().type_("image/jpeg"),
		)
		.unwrap();
		assert_eq!(
			from_file(&file, Options::new(MediaKind::Images, 2).unwrap()).await,
			Err(Error::TooLarge)
		);
		let asset = from_file(&file, Options::new(MediaKind::Images, 3).unwrap())
			.await
			.unwrap();
		assert!(asset.uri.starts_with("blob:"));
		assert_eq!(asset.mime_type.as_deref(), Some("image/jpeg"));
		assert_eq!(asset.size_bytes, Some(3));
		web_sys::Url::revoke_object_url(&asset.uri).unwrap();
	}

	#[wasm_bindgen_test]
	fn native_browser_failures_remain_distinct() {
		for (name, expected) in [
			("NotAllowedError", Error::PermissionDenied),
			("AbortError", Error::Cancelled),
			("NotReadableError", Error::Native),
		] {
			let error =
				web_sys::DomException::new_with_message_and_name("synthetic", name).unwrap();
			assert_eq!(failure(error.as_ref()), expected);
		}
	}
}
