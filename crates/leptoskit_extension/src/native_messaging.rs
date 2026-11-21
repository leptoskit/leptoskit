//! Native messaging (`runtime.sendNativeMessage` / `runtime.connectNative`).
//!
//! Connects the browser extension to a companion native host on Chromium and
//! Firefox, or to the containing macOS / iOS app on Safari. Requires the
//! `"nativeMessaging"` permission.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Error;
use crate::Result;
use crate::js;
use crate::runtime::Port;

/// Whether `runtime.sendNativeMessage` is exposed in this browser.
#[must_use]
pub fn available() -> bool {
	js::has("runtime.sendNativeMessage")
}

/// Sends one message to the named native messaging host (or Safari containing
/// app bundle identifier) and awaits its reply.
///
/// # Errors
///
/// [`Error::Unavailable`] outside a browser or when `"nativeMessaging"` is not
/// granted; [`Error::Browser`] when the native host refuses or isn't installed.
pub async fn send<M: Serialize, R: DeserializeOwned>(application: &str, message: &M) -> Result<R> {
	if !js::has("runtime.sendNativeMessage") {
		return Err(Error::Unavailable("runtime.sendNativeMessage".into()));
	}

	js::call("runtime.sendNativeMessage", &[&application, message]).await
}

/// Opens a long-lived native messaging [`Port`] to `application`.
///
/// # Errors
///
/// [`Error::Unavailable`] outside a browser or when `"nativeMessaging"` is not
/// granted.
pub fn connect(application: &str) -> Result<Port> {
	#[cfg(target_arch = "wasm32")]
	{
		use js_sys::Function;
		use js_sys::Reflect;
		use wasm_bindgen::JsCast;
		use wasm_bindgen::JsValue;

		let runtime = js::object("runtime")?;
		let connect = Reflect::get(&runtime, &JsValue::from_str("connectNative"))
			.ok()
			.and_then(|value| value.dyn_into::<Function>().ok())
			.ok_or_else(|| Error::Unavailable("runtime.connectNative".into()))?;
		let raw = connect
			.call1(&runtime, &JsValue::from_str(application))
			.map_err(|error| Error::Browser(js::message(&error)))?;
		Ok(Port::from_js(application.to_owned(), raw))
	}

	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = application;
		Err(Error::Unavailable("runtime.connectNative".into()))
	}
}
