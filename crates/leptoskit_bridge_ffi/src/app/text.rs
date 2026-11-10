//! Text measured by the embedding host, for platforms whose fonts Rust cannot
//! reach directly. Android draws with its own typefaces, so layout asks the
//! host how wide each label is instead of approximating it.

use std::ffi::c_void;
use std::sync::Arc;

use leptoskit_bridge::Error;
use leptoskit_native::ApproxTextMeasurer;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Direction;
use leptoskit_yoga::Size;

use crate::app::App;

/// A host text measurer and its owned context.
///
/// `measure` lays out `len` bytes of UTF-8 `text` at `font_size` logical
/// points, CSS `font_weight` (100 to 900), and `rtl` direction within `width`
/// logical points (`-1` unbounded, `-2` the widest word), writes the logical
/// width and height to `out[0]` and `out[1]`, and returns `true`. It may run on
/// any thread. `release` runs once, after the last measurement.
#[repr(C)]
pub struct NativeTextMeasurer {
	pub context: *mut c_void,
	pub measure: Option<
		unsafe extern "C" fn(*mut c_void, *const u8, usize, f32, u32, f32, bool, *mut f32) -> bool,
	>,
	pub release: Option<unsafe extern "C" fn(*mut c_void)>,
}

struct HostMeasurer {
	context: usize,
	measure:
		unsafe extern "C" fn(*mut c_void, *const u8, usize, f32, u32, f32, bool, *mut f32) -> bool,
	release: unsafe extern "C" fn(*mut c_void),
}

impl TextMeasurer for HostMeasurer {
	fn measure(&self, text: &str, style: TextStyle, available: Size<AvailableSpace>) -> Size<f32> {
		let width = match available.width {
			AvailableSpace::Definite(width) => width.max(0.0),
			AvailableSpace::MinContent => -2.0,
			AvailableSpace::MaxContent => -1.0,
		};

		let size = self.natural(text, style, width);

		// The host sets lines at the font's own height; the renderer draws
		// each one at the line height, so the label is that many lines tall.
		if style.line_height.is_some()
			&& let Some(size) = size
			&& let Some(line) = self.natural(" ", style, -1.0)
		{
			return style.fit_line_height(size, line.height);
		}

		size.unwrap_or_else(|| {
			// The host reports why it could not measure; layout still needs a
			// size, so it falls back to the approximation for this label.
			ApproxTextMeasurer::new().measure(text, style, available)
		})
	}
}

impl HostMeasurer {
	/// The host's size for `text` at the font's own line height, or `None`
	/// when the host could not measure it.
	fn natural(&self, text: &str, style: TextStyle, width: f32) -> Option<Size<f32>> {
		let weight = u32::from(style.font_weight.value());
		let mut size = [0.0_f32; 2];
		// SAFETY: The context is owned until release, the text borrow outlives
		// the call, and `size` holds the two floats the contract writes.
		let measured = unsafe {
			(self.measure)(
				self.context as *mut c_void,
				text.as_ptr(),
				text.len(),
				style.font_size,
				weight,
				width,
				style.direction == Direction::Rtl,
				size.as_mut_ptr(),
			)
		};

		(measured && size.iter().all(|value| value.is_finite()))
			.then(|| Size::new(size[0].max(0.0), size[1].max(0.0)))
	}
}

impl Drop for HostMeasurer {
	fn drop(&mut self) {
		// SAFETY: This is the last owner; no measurement can follow.
		unsafe { (self.release)(self.context as *mut c_void) };
	}
}

/// Measures the app's text with the host's fonts from now on, replacing the
/// approximation, and lays the current tree out again.
///
/// # Safety
/// App is exclusively borrowed on its UI thread. The context is owned until
/// release, and both callbacks are nonnull and thread-safe. A null callback
/// leaves the context with the caller; a null app releases it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_set_text_measurer(
	app: *mut App,
	measurer: NativeTextMeasurer,
) -> i32 {
	let (Some(measure), Some(release)) = (measurer.measure, measurer.release) else {
		return -(Error::InvalidArgument as i32);
	};
	let host = Arc::new(HostMeasurer {
		context: measurer.context as usize,
		measure,
		release,
	});
	// SAFETY: The embedding host lends the live app for this operation.
	let Some(app) = (unsafe { app.as_mut() }) else {
		return -(Error::InvalidArgument as i32);
	};
	app.host_mut()
		.update_backend(|backend| backend.set_text_measurer(host));
	app.record(Ok(()))
}

#[cfg(test)]
mod tests {
	use std::sync::Mutex;
	use std::sync::atomic::AtomicUsize;
	use std::sync::atomic::Ordering;

	use leptoskit_native::FontWeight;
	use leptoskit_native::NativeBackend;

	use super::*;

	const GUEST: &[u8] = br#"(module
 (memory (export "memory") 1)
 (func (export "leptoskit_main"))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
)"#;

	/// One measurement: the text, font size, weight, width, and direction.
	type Call = (String, f32, u32, f32, bool);

	/// The context a test host measures with: every call and every release.
	#[derive(Default)]
	struct Host {
		calls: Mutex<Vec<Call>>,
		released: AtomicUsize,
	}

	impl Host {
		fn context(&self) -> *mut c_void {
			std::ptr::from_ref(self).cast_mut().cast()
		}

		fn calls(&self) -> Vec<Call> {
			self.calls.lock().unwrap().clone()
		}

		fn released(&self) -> usize {
			self.released.load(Ordering::SeqCst)
		}
	}

	/// Records the call and reports three points per byte and a line of the
	/// font size's height per line of text, or fails for the text `fail`,
	/// and for a space at 13 points.
	unsafe extern "C" fn measure(
		context: *mut c_void,
		text: *const u8,
		len: usize,
		font_size: f32,
		weight: u32,
		width: f32,
		rtl: bool,
		out: *mut f32,
	) -> bool {
		// SAFETY: Each test passes a live host as the context, and the
		// measurer lends `len` bytes and a two-float buffer.
		let (host, text) = unsafe {
			(
				&*context.cast::<Host>(),
				std::str::from_utf8(std::slice::from_raw_parts(text, len)).unwrap(),
			)
		};
		host.calls
			.lock()
			.unwrap()
			.push((text.to_owned(), font_size, weight, width, rtl));

		if text == "fail" || (text == " " && font_size == 13.0) {
			return false;
		}

		// SAFETY: See above.
		unsafe {
			out.write(text.len() as f32 * 3.0);
			out.add(1)
				.write(font_size * text.split('\n').count() as f32);
		}
		true
	}

	unsafe extern "C" fn release(context: *mut c_void) {
		// SAFETY: Each test passes a live host as the context.
		unsafe { &*context.cast::<Host>() }
			.released
			.fetch_add(1, Ordering::SeqCst);
	}

	fn table(host: &Host) -> NativeTextMeasurer {
		NativeTextMeasurer {
			context: host.context(),
			measure: Some(measure),
			release: Some(release),
		}
	}

	#[test]
	fn host_sizes_text_falls_back_on_failure_and_releases_once() {
		let host = Host::default();
		let measurer = HostMeasurer {
			context: host.context() as usize,
			measure,
			release,
		};
		let rtl = TextStyle {
			direction: Direction::Rtl,
			..TextStyle::new(20.0, FontWeight::Bold)
		};
		let unbounded = Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
		assert_eq!(
			measurer.measure("Add one", rtl, unbounded),
			Size::new(21.0, 20.0)
		);
		assert_eq!(
			measurer.measure("fail", rtl, unbounded),
			ApproxTextMeasurer::new().measure("fail", rtl, unbounded)
		);
		drop(measurer);
		assert_eq!(host.released(), 1);
	}

	#[test]
	fn host_receives_each_width_constraint_and_weight() {
		let host = Host::default();
		let measurer = HostMeasurer {
			context: host.context() as usize,
			measure,
			release,
		};
		let cases = [
			(AvailableSpace::Definite(80.0), FontWeight::Regular),
			(AvailableSpace::Definite(-4.0), FontWeight::Medium),
			(AvailableSpace::MinContent, FontWeight::Semibold),
			(AvailableSpace::MaxContent, FontWeight::Bold),
		];

		for (width, weight) in cases {
			measurer.measure(
				"Go",
				TextStyle::new(16.0, weight),
				Size::new(width, AvailableSpace::MaxContent),
			);
		}

		let go = |weight, width| ("Go".to_owned(), 16.0, weight, width, false);
		assert_eq!(
			host.calls(),
			[go(400, 80.0), go(500, 0.0), go(600, -2.0), go(700, -1.0)]
		);
	}

	#[test]
	fn host_receives_every_css_weight() {
		let host = Host::default();
		let measurer = HostMeasurer {
			context: host.context() as usize,
			measure,
			release,
		};
		let unbounded = Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent);

		for weight in FontWeight::ALL {
			measurer.measure("Go", TextStyle::new(16.0, weight), unbounded);
		}

		let weights = host
			.calls()
			.into_iter()
			.map(|call| call.2)
			.collect::<Vec<_>>();
		assert_eq!(weights, [100, 200, 300, 400, 500, 600, 700, 800, 900]);
	}

	#[test]
	fn host_sizes_take_the_line_height_per_line() {
		let host = Host::default();
		let measurer = HostMeasurer {
			context: host.context() as usize,
			measure,
			release,
		};
		let unbounded = Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
		let style = |font_size| {
			TextStyle {
				line_height: Some(30.0),
				..TextStyle::new(font_size, FontWeight::Regular)
			}
		};

		// Two lines of 20 points become two lines of 30.
		assert_eq!(
			measurer.measure("one\ntwo", style(20.0), unbounded),
			Size::new(21.0, 60.0)
		);
		// Without a natural line to count by, the host's size stands.
		assert_eq!(
			measurer.measure("one\ntwo", style(13.0), unbounded),
			Size::new(21.0, 26.0)
		);
		// A failed measurement still falls back to the approximation, which
		// takes the line height too.
		assert_eq!(
			measurer.measure("fail", style(20.0), unbounded),
			Size::new(
				ApproxTextMeasurer::new()
					.measure("fail", style(20.0), unbounded)
					.width,
				30.0
			)
		);
	}

	#[test]
	fn apps_lay_text_out_with_the_host_measurer_until_dropped() {
		let host = Host::default();
		let mut app = App::new(GUEST).unwrap();
		// SAFETY: The app is live and exclusively borrowed for the call.
		assert_eq!(
			unsafe { lk_app_set_text_measurer(&raw mut app, table(&host)) },
			0
		);
		let style = TextStyle::new(20.0, FontWeight::Regular);
		let unbounded = Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
		let measured = app.with_host(|wasm| {
			wasm.surface()
				.backend()
				.text_measurer()
				.measure("Add one", style, unbounded)
		});
		assert_eq!(measured, Size::new(21.0, 20.0));
		assert_eq!(host.released(), 0);

		drop(app);
		assert_eq!(host.released(), 1);
	}

	#[test]
	fn invalid_tables_keep_their_context_and_null_apps_release_it() {
		let host = Host::default();
		let invalid = NativeTextMeasurer {
			measure: None,
			..table(&host)
		};
		// SAFETY: Invalid tables are rejected before any app access.
		let status = unsafe { lk_app_set_text_measurer(std::ptr::null_mut(), invalid) };
		assert_eq!(status, -(Error::InvalidArgument as i32));
		assert_eq!(host.released(), 0, "the caller keeps an unaccepted context");
		// SAFETY: A null app releases the accepted context.
		let status = unsafe { lk_app_set_text_measurer(std::ptr::null_mut(), table(&host)) };
		assert_eq!(status, -(Error::InvalidArgument as i32));
		assert_eq!(host.released(), 1);
	}
}
