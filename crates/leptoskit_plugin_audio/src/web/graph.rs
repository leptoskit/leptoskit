//! Web Audio node plumbing: parameter ramps, filter slots, buffer sources,
//! timers and fetches.

use std::time::Duration;

use js_sys::Function;
use leptoskit_bridge::Error;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use web_sys::AudioBuffer;
use web_sys::AudioBufferSourceNode;
use web_sys::AudioContext;
use web_sys::AudioParam;
use web_sys::AudioScheduledSourceNode;
use web_sys::BiquadFilterNode;
use web_sys::BiquadFilterType;

use crate::Filter;
use crate::FilterKind;
use crate::MIN_CUTOFF;

/// The shortest audible change, as in the native engine.
pub const MIN_RAMP: f64 = 0.005;
/// The highest cutoff, as a fraction of the sample rate.
const CEILING: f32 = 0.45;
/// A filter with no peak at its cutoff, in the decibels Web Audio's
/// low-pass and high-pass filters take their resonance in.
const PLAIN_RESONANCE_DB: f32 = -3.01;
/// Decibels per decade of linear gain.
const DECIBELS: f32 = 20.0;
const NOT_FOUND: u16 = 404;

pub fn native(_: JsValue) -> Error {
	Error::Native
}

pub fn seconds(duration: Duration) -> f64 {
	duration.as_secs_f64()
}

/// Moves a parameter from wherever it is now to `value` over `time`.
pub fn ramp(param: &AudioParam, value: f32, now: f64, time: f64) {
	let _ = param.cancel_scheduled_values(now);
	let _ = param.set_value_at_time(param.value(), now);
	let _ = param.linear_ramp_to_value_at_time(value, now + time.max(MIN_RAMP));
}

/// Like [`ramp`], in equal ratios: for frequencies.
fn glide(param: &AudioParam, value: f32, now: f64, time: f64) {
	let _ = param.cancel_scheduled_values(now);
	let _ = param.set_value_at_time(param.value(), now);
	let _ = param.exponential_ramp_to_value_at_time(value, now + time.max(MIN_RAMP));
}

/// A filter slot that starts out doing nothing: a peaking filter at 0 dB.
pub fn transparent(context: &AudioContext) -> Result<BiquadFilterNode, Error> {
	let filter = context.create_biquad_filter().map_err(native)?;
	filter.set_type(BiquadFilterType::Peaking);
	filter.gain().set_value(0.0);
	Ok(filter)
}

/// Sets a slot. Removing a filter glides it open and leaves it there.
pub fn set_filter(node: &BiquadFilterNode, filter: Option<Filter>, rate: f32, now: f64, time: f64) {
	let ceiling = rate * CEILING;

	let Some(filter) = filter else {
		let open = match node.type_() {
			BiquadFilterType::Lowpass => ceiling,
			BiquadFilterType::Highpass => MIN_CUTOFF,
			_ => return,
		};

		glide(&node.frequency(), open, now, time);

		return;
	};

	let (kind, open) = match filter.kind {
		FilterKind::LowPass => (BiquadFilterType::Lowpass, ceiling),
		FilterKind::HighPass => (BiquadFilterType::Highpass, MIN_CUTOFF),
	};

	// A new shape starts open, where it changes nothing, and closes in.
	if node.type_() != kind {
		node.set_type(kind);
		node.frequency().set_value(open);
		node.q().set_value(PLAIN_RESONANCE_DB);
	}

	glide(&node.frequency(), filter.cutoff.min(ceiling), now, time);
	ramp(&node.q(), DECIBELS * filter.resonance.log10(), now, time);
}

/// Starts a buffer `offset` seconds in, playing into `into`.
pub fn start_buffer(
	context: &AudioContext,
	buffer: &AudioBuffer,
	into: &BiquadFilterNode,
	looping: bool,
	offset: f64,
	ended: &Closure<dyn FnMut()>,
) -> Result<AudioBufferSourceNode, Error> {
	let node = context.create_buffer_source().map_err(native)?;
	node.set_buffer(Some(buffer));
	node.set_loop(looping);
	node.connect_with_audio_node(into).map_err(native)?;
	scheduled(&node).set_onended(Some(ended.as_ref().unchecked_ref()));
	node.start_with_when_and_grain_offset(0.0, offset)
		.map_err(native)?;
	Ok(node)
}

/// Stops a buffer at `end` without reporting it as having ended.
pub fn halt(node: &AudioBufferSourceNode, end: f64) {
	scheduled(node).set_onended(None::<&Function>);
	let _ = scheduled(node).stop_with_when(end);
}

/// The node's scheduling methods, which the buffer source's own copies
/// deprecate.
pub fn scheduled(node: &AudioBufferSourceNode) -> &AudioScheduledSourceNode {
	node
}

/// Runs `then` after `seconds`.
pub fn later(seconds: f64, then: impl FnOnce() + 'static) {
	let Some(window) = web_sys::window() else {
		return;
	};
	let closure = Closure::once_into_js(then);
	let milliseconds = Duration::from_secs_f64(seconds).as_millis();
	let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
		closure.unchecked_ref(),
		i32::try_from(milliseconds).unwrap_or(i32::MAX),
	);
}

pub async fn fetch(url: &str) -> Result<web_sys::Response, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	let response: web_sys::Response = JsFuture::from(window.fetch_with_str(url))
		.await
		.map_err(native)?
		.unchecked_into();

	match (response.ok(), response.status()) {
		(true, _) => Ok(response),
		(false, NOT_FOUND) => Err(Error::Unavailable),
		(false, _) => Err(Error::Native),
	}
}
