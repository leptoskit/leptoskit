//! Direct Web Share API, including Level 2 file sharing. Call `share`
//! synchronously from a trusted user action. The browser controls its UI and
//! completion timing; it never says which destination was chosen, so a
//! completed share is [`ShareResult::Unavailable`] and a cancelled one is
//! [`ShareResult::Dismissed`]. Dropping `ShareRequest` retires the caller, but
//! admission remains busy until the browser promise settles because its UI
//! cannot be dismissed.
//!
//! Browsers can share in-memory files ([`FileSource::Bytes`]) only. A file in
//! the app's native files directory ([`FileSource::App`]) has no browser
//! equivalent; read it first and share its bytes. Unsupported content returns
//! `Unavailable` instead of downloading files or opening `mailto:`.
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use futures::channel::oneshot;
use js_sys::Array;
use js_sys::Function;
use js_sys::Object;
use js_sys::Promise;
use js_sys::Reflect;
use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use crate::Content;
use crate::Error;
use crate::FileSource;
use crate::ShareResult;

thread_local! { static BUSY: Cell<bool> = const { Cell::new(false) }; }
struct Admission;
impl Admission {
	fn acquire() -> Result<Self, Error> {
		if BUSY.replace(true) {
			Err(Error::Busy)
		} else {
			Ok(Self)
		}
	}
}
impl Drop for Admission {
	fn drop(&mut self) {
		BUSY.set(false);
	}
}

/// One browser request; dropping it cannot dismiss the OS share UI.
pub struct ShareRequest(oneshot::Receiver<Result<ShareResult, Error>>);
impl Future for ShareRequest {
	type Output = Result<ShareResult, Error>;

	fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
		Pin::new(&mut self.0)
			.poll(context)
			.map(|reply| reply.unwrap_or(Err(Error::Cancelled)))
	}
}

/// Checks secure context and API presence without opening UI. Permissions
/// policy, activation and content support may still reject a subsequent call.
pub fn is_available() -> bool {
	let Some(window) = web_sys::window() else {
		return false;
	};
	window.is_secure_context() && share_function(window.navigator().as_ref()).is_ok()
}

/// Immediately requests native browser share UI, preserving user activation.
/// Await the returned request to observe API completion, never delivery.
pub fn share(content: &Content) -> Result<ShareRequest, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;

	if !window.is_secure_context() {
		return Err(Error::PermissionDenied);
	}

	begin(window.navigator().as_ref(), content)
}

fn begin(target: &JsValue, content: &Content) -> Result<ShareRequest, Error> {
	let function = share_function(target)?;
	let data = share_data(content)?;

	// Level 2 browsers report content they cannot share. Files need this
	// check: a browser without `canShare` predates file sharing.
	match function_named(target, "canShare")? {
		Some(can_share) => {
			if !can_share
				.call1(target, &data)
				.map_err(|error| failure(&error))?
				.is_truthy()
			{
				return Err(Error::Unavailable);
			}
		}
		None if !content.shared_files().is_empty() => return Err(Error::Unavailable),
		None => {}
	}

	let admission = Admission::acquire()?;
	let promise = function
		.call1(target, &data)
		.map_err(|error| failure(&error))?
		.dyn_into::<Promise>()
		.map_err(|_| Error::Native)?;
	let (sender, receiver) = oneshot::channel();

	wasm_bindgen_futures::spawn_local(async move {
		let result = match JsFuture::from(promise).await {
			Ok(_) => Ok(ShareResult::Unavailable),
			Err(error) if dom_name(&error).as_deref() == Some("AbortError") => {
				Ok(ShareResult::Dismissed)
			}
			Err(error) => Err(failure(&error)),
		};
		drop(admission);
		let _ = sender.send(result);
	});
	Ok(ShareRequest(receiver))
}

/// Builds the `ShareData` dictionary. The browser's single `title` takes the
/// title, or the subject when there is no title.
fn share_data(content: &Content) -> Result<Object, Error> {
	let data = Object::new();

	for (field, value) in [
		("url", content.shared_url()),
		("text", content.shared_text()),
		("title", content.title().or(content.subject())),
	] {
		if let Some(value) = value {
			set(&data, field, &value.into())?;
		}
	}

	if !content.shared_files().is_empty() {
		let files = Array::new();

		for file in content.shared_files() {
			let FileSource::Bytes(bytes) = file.source() else {
				return Err(Error::Unavailable);
			};
			let parts = Array::of1(&Uint8Array::from(bytes.as_slice()));
			let options = web_sys::FilePropertyBag::new();
			options.set_type(file.mime().unwrap_or_default());
			let name = file.name().ok_or(Error::InvalidArgument)?;
			let file =
				web_sys::File::new_with_u8_array_sequence_and_options(&parts, name, &options)
					.map_err(|error| failure(&error))?;
			files.push(file.as_ref());
		}

		set(&data, "files", &files)?;
	}

	Ok(data)
}

fn set(data: &Object, field: &str, value: &JsValue) -> Result<(), Error> {
	if Reflect::set(data, &field.into(), value).map_err(|error| failure(&error))? {
		Ok(())
	} else {
		Err(Error::Native)
	}
}

fn share_function(target: &JsValue) -> Result<Function, Error> {
	function_named(target, "share")?.ok_or(Error::Unavailable)
}

fn function_named(target: &JsValue, name: &str) -> Result<Option<Function>, Error> {
	Ok(Reflect::get(target, &name.into())
		.map_err(|error| failure(&error))?
		.dyn_into::<Function>()
		.ok())
}

fn dom_name(error: &JsValue) -> Option<String> {
	error
		.dyn_ref::<web_sys::DomException>()
		.map(web_sys::DomException::name)
}

fn failure(error: &JsValue) -> Error {
	match dom_name(error).as_deref() {
		Some("AbortError") => Error::Cancelled,
		Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied,
		Some("InvalidStateError") => Error::Busy,
		Some("NotSupportedError" | "DataError") => Error::Unavailable,

		_ if error.is_instance_of::<js_sys::TypeError>() => Error::InvalidArgument,
		_ => Error::Native,
	}
}

#[cfg(test)]
mod tests {
	use std::cell::RefCell;
	use std::rc::Rc;

	use wasm_bindgen::closure::Closure;
	use wasm_bindgen_test::wasm_bindgen_test;

	use super::*;
	use crate::File;
	wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

	/// Settles the spawned promise waiter before checking admission.
	async fn settle() {
		for _ in 0..2 {
			JsFuture::from(Promise::resolve(&JsValue::NULL))
				.await
				.unwrap();
		}
	}

	/// A navigator whose `share` records its data and returns `promise`, and
	/// whose optional `canShare` answers `can_share`.
	fn navigator(
		promise: Promise,
		can_share: Option<bool>,
	) -> (Object, Rc<RefCell<Option<JsValue>>>) {
		let fake = Object::new();
		let seen = Rc::new(RefCell::new(None));
		let recorded = seen.clone();
		let share = Closure::<dyn FnMut(JsValue) -> Promise>::new(move |data: JsValue| {
			*recorded.borrow_mut() = Some(data);
			promise.clone()
		});
		Reflect::set(&fake, &"share".into(), share.as_ref()).unwrap();
		share.forget();

		if let Some(answer) = can_share {
			let check = Closure::<dyn FnMut(JsValue) -> bool>::new(move |_| answer);
			Reflect::set(&fake, &"canShare".into(), check.as_ref()).unwrap();
			check.forget();
		}

		(fake, seen)
	}

	fn field(data: &JsValue, name: &str) -> JsValue {
		Reflect::get(data, &name.into()).unwrap()
	}

	#[wasm_bindgen_test]
	async fn dropped_caller_keeps_browser_admission_until_promise_settles() {
		let mut resolve = None;
		let promise = Promise::new(&mut |yes, _| resolve = Some(yes));
		let (fake, seen) = navigator(promise, None);
		let content = Content::text("synthetic fixture").unwrap();
		drop(begin(&fake, &content).unwrap());
		assert_eq!(
			field(seen.borrow().as_ref().unwrap(), "text")
				.as_string()
				.unwrap(),
			"synthetic fixture"
		);
		assert!(matches!(begin(&fake, &content), Err(Error::Busy)));
		resolve.unwrap().call0(&JsValue::NULL).unwrap();
		settle().await;
		assert!(!BUSY.get());
	}

	#[wasm_bindgen_test]
	async fn completion_is_unavailable_and_abort_is_dismissed() {
		let (fake, _) = navigator(Promise::resolve(&JsValue::UNDEFINED), Some(true));
		let content = Content::url("https://example.test").unwrap();
		assert_eq!(
			begin(&fake, &content).unwrap().await,
			Ok(ShareResult::Unavailable)
		);

		let abort =
			web_sys::DomException::new_with_message_and_name("synthetic", "AbortError").unwrap();
		let (fake, _) = navigator(Promise::reject(&abort), Some(true));
		assert_eq!(
			begin(&fake, &content).unwrap().await,
			Ok(ShareResult::Dismissed)
		);

		let denied =
			web_sys::DomException::new_with_message_and_name("synthetic", "NotAllowedError")
				.unwrap();
		let (fake, _) = navigator(Promise::reject(&denied), Some(true));
		assert_eq!(
			begin(&fake, &content).unwrap().await,
			Err(Error::PermissionDenied)
		);
		settle().await;
		assert!(!BUSY.get());
	}

	#[wasm_bindgen_test]
	async fn files_become_browser_files_with_names_and_types() {
		let (fake, seen) = navigator(Promise::resolve(&JsValue::UNDEFINED), Some(true));
		let content = Content::files([File::bytes("photo.png", [1, 2, 3])
			.unwrap()
			.with_mime("image/png")
			.unwrap()])
		.unwrap()
		.with_text("caption")
		.unwrap()
		.with_subject("subject")
		.unwrap();
		begin(&fake, &content).unwrap().await.unwrap();
		let data = seen.borrow().clone().unwrap();
		assert_eq!(field(&data, "text").as_string().unwrap(), "caption");
		// Without a title the subject becomes the browser's title.
		assert_eq!(field(&data, "title").as_string().unwrap(), "subject");
		assert!(field(&data, "url").is_undefined());
		let files: Array = field(&data, "files").dyn_into().unwrap();
		assert_eq!(files.length(), 1);
		let file: web_sys::File = files.get(0).dyn_into().unwrap();
		assert_eq!(file.name(), "photo.png");
		assert_eq!(file.type_(), "image/png");
		assert!((file.size() - 3.0).abs() < f64::EPSILON);
	}

	#[wasm_bindgen_test]
	fn unsupported_content_is_unavailable_before_opening_ui() {
		let files = Content::files([File::bytes("a.txt", *b"a").unwrap()]).unwrap();
		let (refusing, seen) = navigator(Promise::resolve(&JsValue::UNDEFINED), Some(false));
		assert!(matches!(begin(&refusing, &files), Err(Error::Unavailable)));
		let (level_one, _) = navigator(Promise::resolve(&JsValue::UNDEFINED), None);
		assert!(matches!(begin(&level_one, &files), Err(Error::Unavailable)));
		let app = Content::files([File::app("a.txt").unwrap()]).unwrap();
		let (accepting, _) = navigator(Promise::resolve(&JsValue::UNDEFINED), Some(true));
		assert!(matches!(begin(&accepting, &app), Err(Error::Unavailable)));
		assert!(seen.borrow().is_none());
		assert!(!BUSY.get());
	}

	#[wasm_bindgen_test]
	fn browser_errors_are_not_delivery_results() {
		for (name, expected) in [
			("AbortError", Error::Cancelled),
			("InvalidStateError", Error::Busy),
			("NotAllowedError", Error::PermissionDenied),
			("DataError", Error::Unavailable),
		] {
			let error =
				web_sys::DomException::new_with_message_and_name("synthetic", name).unwrap();
			assert_eq!(failure(error.as_ref()), expected);
		}
		assert!(matches!(
			share_function(&Object::new()),
			Err(Error::Unavailable)
		));
	}
}
