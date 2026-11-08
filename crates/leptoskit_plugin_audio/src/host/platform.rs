//! What differs per operating system: where bundled assets live, and the
//! audio session.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_bridge::AppContext;
use leptoskit_bridge::Error;

use super::source::Encoded;
use crate::Event;
use crate::Session;

/// Events waiting for the guest, shared with the operating system's
/// notification callbacks.
pub type Events = Arc<Mutex<VecDeque<Event>>>;

/// The most events kept; the oldest go first.
pub const MAX_EVENTS: usize = 1_024;

pub fn push(events: &Events, event: Event) {
	if let Ok(mut events) = events.lock() {
		if events.len() == MAX_EVENTS {
			events.pop_front();
		}

		events.push_back(event);
	}
}

/// Opens an asset of the app bundle: Android reads it from the APK, everyone
/// else from the `assets` folder beside the executable (on macOS, in the
/// bundle's `Resources`).
pub fn asset(context: &AppContext, path: &str) -> Result<Encoded, Error> {
	#[cfg(target_os = "android")]
	{
		android::asset(context, path)
	}
	#[cfg(not(target_os = "android"))]
	{
		let _ = context;
		let executable = std::env::current_exe().map_err(|_| Error::Unavailable)?;
		let directory = executable.parent().ok_or(Error::Unavailable)?;

		let root = match directory.parent() {
			Some(contents)
				if cfg!(target_os = "macos") && directory.ends_with("Contents/MacOS") =>
			{
				contents.join("Resources/assets")
			}
			_ => directory.join("assets"),
		};

		Ok(Encoded::File(root.join(path)))
	}
}

/// Platform state the engine keeps: audio focus or notification observers.
#[derive(Debug, Default)]
pub struct State {
	#[cfg(target_os = "ios")]
	observers: Option<apple::Observers>,
	#[cfg(target_os = "android")]
	focus: Option<android::Focus>,
}

/// Prepares the platform before the first output opens.
#[cfg_attr(not(target_os = "android"), allow(clippy::unnecessary_wraps))]
pub fn prepare(context: &AppContext) -> Result<(), Error> {
	#[cfg(target_os = "android")]
	{
		android::prepare(context)
	}
	#[cfg(not(target_os = "android"))]
	{
		let _ = context;
		Ok(())
	}
}

/// Applies the audio session. Desktops have none.
#[cfg_attr(
	not(any(target_os = "ios", target_os = "android")),
	allow(clippy::unnecessary_wraps)
)]
pub fn configure(
	context: &AppContext,
	session: Session,
	events: &Events,
	state: &mut State,
) -> Result<(), Error> {
	#[cfg(target_os = "ios")]
	{
		let _ = context;
		apple::configure(session)?;

		if state.observers.is_none() {
			state.observers = Some(apple::Observers::new(events));
		}

		Ok(())
	}
	#[cfg(target_os = "android")]
	{
		let _ = events;
		android::configure(context, session, &mut state.focus)
	}
	#[cfg(not(any(target_os = "ios", target_os = "android")))]
	{
		let _ = (context, session, events, state);
		Ok(())
	}
}

#[cfg(target_os = "ios")]
mod apple {
	use std::ptr::NonNull;

	use block2::RcBlock;
	use objc2::rc::Retained;
	use objc2::runtime::NSObjectProtocol;
	use objc2::runtime::ProtocolObject;
	use objc2_avf_audio::AVAudioSession;
	use objc2_avf_audio::AVAudioSessionCategoryAmbient;
	use objc2_avf_audio::AVAudioSessionCategoryOptions;
	use objc2_avf_audio::AVAudioSessionCategoryPlayback;
	use objc2_avf_audio::AVAudioSessionInterruptionNotification;
	use objc2_avf_audio::AVAudioSessionInterruptionType;
	use objc2_avf_audio::AVAudioSessionInterruptionTypeKey;
	use objc2_foundation::NSNotification;
	use objc2_foundation::NSNotificationCenter;
	use objc2_foundation::NSNumber;

	use super::*;
	use crate::Category;

	pub fn configure(session: Session) -> Result<(), Error> {
		// SAFETY: The shared session is thread-safe, and the category
		// constants are immutable framework statics.
		let (shared, category) = unsafe {
			(
				AVAudioSession::sharedInstance(),
				match session.category {
					Category::Ambient => AVAudioSessionCategoryAmbient,
					Category::Playback => AVAudioSessionCategoryPlayback,
				},
			)
		};

		let category = category.ok_or(Error::Unavailable)?;
		let mut options = AVAudioSessionCategoryOptions(0);

		if session.category == Category::Playback {
			if session.mix_with_others {
				options.0 |= AVAudioSessionCategoryOptions::MixWithOthers.0;
			}

			if session.duck_others {
				options.0 |= AVAudioSessionCategoryOptions::DuckOthers.0;
			}
		}

		// SAFETY: Valid category and options for this session.
		unsafe { shared.setCategory_withOptions_error(category, options) }
			.map_err(|_| Error::Native)?;
		// SAFETY: Activating the shared session has no other precondition.
		unsafe { shared.setActive_error(true) }.map_err(|_| Error::Busy)
	}

	/// Reports interruptions as [`Event`]s; cpal stops and restarts the
	/// stream itself.
	#[derive(Debug)]
	pub struct Observers(Retained<ProtocolObject<dyn NSObjectProtocol>>);

	// SAFETY: The token is only passed back to `removeObserver:`, which
	// `NSNotificationCenter` allows from any thread.
	unsafe impl Send for Observers {}

	impl Observers {
		pub fn new(events: &Events) -> Self {
			let events = Arc::clone(events);
			let block = RcBlock::new(move |notification: NonNull<NSNotification>| {
				// SAFETY: The notification centre passes a live notification.
				let notification = unsafe { notification.as_ref() };

				report(&events, kind(notification));
			});
			let center = NSNotificationCenter::defaultCenter();
			// SAFETY: The name is a framework constant; a nil object and
			// queue deliver every session's notification on the posting
			// thread, and the block only locks the event queue.
			let token = unsafe {
				center.addObserverForName_object_queue_usingBlock(
					AVAudioSessionInterruptionNotification,
					None,
					None,
					&block,
				)
			};
			Self(token)
		}
	}

	impl Drop for Observers {
		fn drop(&mut self) {
			// SAFETY: The token came from this notification centre.
			unsafe { NSNotificationCenter::defaultCenter().removeObserver(self.0.as_ref()) };
		}
	}

	fn report(events: &Events, kind: Option<AVAudioSessionInterruptionType>) {
		match kind {
			Some(AVAudioSessionInterruptionType::Began) => push(events, Event::Interrupted),
			Some(AVAudioSessionInterruptionType::Ended) => push(events, Event::Resumed),
			_ => {}
		}
	}

	fn kind(notification: &NSNotification) -> Option<AVAudioSessionInterruptionType> {
		// SAFETY: A framework constant.
		let key = unsafe { AVAudioSessionInterruptionTypeKey }?;
		let value = notification.userInfo()?.objectForKey(key)?;
		Some(AVAudioSessionInterruptionType(
			value.downcast::<NSNumber>().ok()?.unsignedIntegerValue(),
		))
	}
}

#[cfg(target_os = "android")]
mod android {
	use std::sync::OnceLock;

	use jni::JNIEnv;
	use jni::JavaVM;
	use jni::objects::GlobalRef;
	use jni::objects::JObject;
	use jni::objects::JValue;
	use jni::objects::JValueGen;
	use leptoskit_bridge::PlatformContext;

	use super::*;
	use crate::Category;

	/// The most bytes read from one APK asset: 64 MiB.
	const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
	const USAGE_GAME: i32 = 14;
	const CONTENT_TYPE_MUSIC: i32 = 2;
	const AUDIOFOCUS_GAIN: i32 = 1;
	const AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK: i32 = 3;
	const AUDIOFOCUS_REQUEST_GRANTED: i32 = 1;

	/// The context cpal's `AAudio` backend reads; kept for the process.
	static CONTEXT: OnceLock<PlatformContext> = OnceLock::new();

	/// A held audio-focus request, abandoned when replaced or dropped.
	#[derive(Debug)]
	pub struct Focus {
		vm: JavaVM,
		context: GlobalRef,
		request: GlobalRef,
	}

	impl Drop for Focus {
		fn drop(&mut self) {
			if let Ok(mut env) = self.vm.attach_current_thread_permanently() {
				let _ = abandon(&mut env, self.context.as_obj(), self.request.as_obj());
			}
		}
	}

	fn handles(context: &AppContext) -> Result<(JavaVM, &PlatformContext), Error> {
		let platform = context.platform().ok_or(Error::Unavailable)?;
		let (vm, _) = platform.android_handles().ok_or(Error::Unavailable)?;
		// SAFETY: The Android host lends its process `JavaVM*` for the
		// lifetime of `PlatformContext`.
		let vm = unsafe { JavaVM::from_raw(vm.cast()) }.map_err(|_| Error::Unavailable)?;
		Ok((vm, platform))
	}

	fn context_object(platform: &PlatformContext) -> Result<JObject<'static>, Error> {
		let (_, context) = platform.android_handles().ok_or(Error::Unavailable)?;
		// SAFETY: `context` is the host's JNI global reference, kept alive by
		// `PlatformContext`; `JObject` borrows it without deleting it.
		Ok(unsafe { JObject::from_raw(context.cast()) })
	}

	/// Hands cpal the process's Java context, once.
	pub fn prepare(context: &AppContext) -> Result<(), Error> {
		let platform = context.platform().ok_or(Error::Unavailable)?;
		let (vm, object) = platform.android_handles().ok_or(Error::Unavailable)?;

		if CONTEXT.set(platform.clone()).is_ok() {
			// SAFETY: Both handles stay valid for the process: the clone in
			// `CONTEXT` is never dropped, and nothing else initializes it.
			unsafe { ndk_context::initialize_android_context(vm, object) };
		}

		Ok(())
	}

	/// Reads a whole APK asset; the engine streams from these bytes.
	pub fn asset(context: &AppContext, path: &str) -> Result<Encoded, Error> {
		let (vm, platform) = handles(context)?;
		let mut env = vm
			.attach_current_thread_permanently()
			.map_err(|_| Error::Native)?;
		let object = context_object(platform)?;
		let bytes = env
			.with_local_frame(16, |env| -> jni::errors::Result<Result<Vec<u8>, Error>> {
				Ok(read_asset(env, &object, path))
			})
			.map_err(|_| Error::Native)??;
		Ok(Encoded::Bytes(bytes.into()))
	}

	/// Reads through `AssetManager.open` 64 KiB at a time. Every JNI step can
	/// throw, and each failure clears the Java exception before returning.
	fn read_asset(
		env: &mut JNIEnv<'_>,
		context: &JObject<'_>,
		path: &str,
	) -> Result<Vec<u8>, Error> {
		let assets = env
			.call_method(
				context,
				"getAssets",
				"()Landroid/content/res/AssetManager;",
				&[],
			)
			.and_then(JValueGen::l)
			.map_err(|_| clear(env))?;
		let name = env.new_string(path).map_err(|_| clear(env))?;
		let stream = env
			.call_method(
				&assets,
				"open",
				"(Ljava/lang/String;)Ljava/io/InputStream;",
				&[JValue::Object(&name)],
			)
			.and_then(JValueGen::l)
			.map_err(|_| {
				clear(env);
				Error::Unavailable
			})?;
		let chunk = env.new_byte_array(64 * 1024).map_err(|_| clear(env))?;
		let mut bytes = Vec::new();
		let mut buffer = vec![0_i8; 64 * 1024];

		let result = loop {
			let Ok(read) = env
				.call_method(&stream, "read", "([B)I", &[JValue::Object(&chunk)])
				.and_then(JValueGen::i)
			else {
				break Err(clear(env));
			};

			if read < 0 {
				break Ok(());
			}

			let read = read as usize;

			if bytes.len() + read > MAX_ASSET_BYTES {
				break Err(Error::TooLarge);
			}

			if env
				.get_byte_array_region(&chunk, 0, &mut buffer[..read])
				.is_err()
			{
				break Err(clear(env));
			}

			bytes.extend(buffer[..read].iter().map(|&byte| byte as u8));
		};

		let _ = env.call_method(&stream, "close", "()V", &[]);
		clear(env);
		result.map(|()| bytes)
	}

	/// Playback asks for audio focus; ambient sound, and playback that mixes
	/// with others, gives it back.
	pub fn configure(
		context: &AppContext,
		session: Session,
		focus: &mut Option<Focus>,
	) -> Result<(), Error> {
		*focus = None;

		if session.category == Category::Ambient || session.mix_with_others {
			return Ok(());
		}

		let (vm, platform) = handles(context)?;
		let object = context_object(platform)?;
		let gain = if session.duck_others {
			AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK
		} else {
			AUDIOFOCUS_GAIN
		};

		let (context, request) = {
			let mut env = vm
				.attach_current_thread_permanently()
				.map_err(|_| Error::Native)?;
			env.with_local_frame(16, |env| -> jni::errors::Result<Result<_, Error>> {
				Ok(request_focus(env, &object, gain))
			})
			.map_err(|_| Error::Native)??
		};
		*focus = Some(Focus {
			vm,
			context,
			request,
		});
		Ok(())
	}

	fn request_focus(
		env: &mut JNIEnv<'_>,
		context: &JObject<'_>,
		gain: i32,
	) -> Result<(GlobalRef, GlobalRef), Error> {
		let attributes = env
			.new_object("android/media/AudioAttributes$Builder", "()V", &[])
			.map_err(|_| clear(env))?;
		env.call_method(
			&attributes,
			"setUsage",
			"(I)Landroid/media/AudioAttributes$Builder;",
			&[JValue::Int(USAGE_GAME)],
		)
		.map_err(|_| clear(env))?;
		env.call_method(
			&attributes,
			"setContentType",
			"(I)Landroid/media/AudioAttributes$Builder;",
			&[JValue::Int(CONTENT_TYPE_MUSIC)],
		)
		.map_err(|_| clear(env))?;
		let attributes = env
			.call_method(
				&attributes,
				"build",
				"()Landroid/media/AudioAttributes;",
				&[],
			)
			.and_then(JValueGen::l)
			.map_err(|_| clear(env))?;
		let builder = env
			.new_object(
				"android/media/AudioFocusRequest$Builder",
				"(I)V",
				&[JValue::Int(gain)],
			)
			.map_err(|_| clear(env))?;
		env.call_method(
			&builder,
			"setAudioAttributes",
			"(Landroid/media/AudioAttributes;)Landroid/media/AudioFocusRequest$Builder;",
			&[JValue::Object(&attributes)],
		)
		.map_err(|_| clear(env))?;
		let request = env
			.call_method(
				&builder,
				"build",
				"()Landroid/media/AudioFocusRequest;",
				&[],
			)
			.and_then(JValueGen::l)
			.map_err(|_| clear(env))?;
		let manager = audio_manager(env, context)?;
		let granted = env
			.call_method(
				&manager,
				"requestAudioFocus",
				"(Landroid/media/AudioFocusRequest;)I",
				&[JValue::Object(&request)],
			)
			.and_then(JValueGen::i)
			.map_err(|_| clear(env))?;

		if granted != AUDIOFOCUS_REQUEST_GRANTED {
			// A call in progress, for instance.
			return Err(Error::Busy);
		}

		let context = env.new_global_ref(context).map_err(|_| clear(env))?;
		let request = env.new_global_ref(&request).map_err(|_| clear(env))?;
		Ok((context, request))
	}

	fn audio_manager<'local>(
		env: &mut JNIEnv<'local>,
		context: &JObject<'_>,
	) -> Result<JObject<'local>, Error> {
		let service = env.new_string("audio").map_err(|_| clear(env))?;
		env.call_method(
			context,
			"getSystemService",
			"(Ljava/lang/String;)Ljava/lang/Object;",
			&[JValue::Object(&service)],
		)
		.and_then(JValueGen::l)
		.map_err(|_| clear(env))
	}

	fn abandon(
		env: &mut JNIEnv<'_>,
		context: &JObject<'_>,
		request: &JObject<'_>,
	) -> Result<(), Error> {
		let manager = audio_manager(env, context)?;
		env.call_method(
			&manager,
			"abandonAudioFocusRequest",
			"(Landroid/media/AudioFocusRequest;)I",
			&[JValue::Object(request)],
		)
		.map_err(|_| clear(env))?;
		Ok(())
	}

	fn clear(env: &mut JNIEnv<'_>) -> Error {
		if env.exception_check().unwrap_or(false) {
			let _ = env.exception_clear();
		}

		Error::Native
	}
}

#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod tests {
	use super::*;

	#[test]
	fn assets_sit_beside_the_executable_and_desktops_have_no_session() {
		let context = AppContext::new(std::env::temp_dir(), std::env::temp_dir());
		let Ok(Encoded::File(path)) = asset(&context, "music/hunt.ogg") else {
			panic!("not a file");
		};
		let executable = std::env::current_exe().unwrap();
		assert_eq!(
			path,
			executable.parent().unwrap().join("assets/music/hunt.ogg")
		);

		let events = Events::default();
		let mut state = State::default();
		assert_eq!(prepare(&context), Ok(()));
		assert_eq!(
			configure(&context, Session::default(), &events, &mut state),
			Ok(())
		);

		for _ in 0..=MAX_EVENTS {
			push(&events, Event::Resumed);
		}

		assert_eq!(events.lock().unwrap().len(), MAX_EVENTS);
	}
}
