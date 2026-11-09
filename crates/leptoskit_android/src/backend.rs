//! The JNI boundary owns global references, never a retained `JNIEnv`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;
use std::rc::Weak;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering;

use jni::JNIEnv;
use jni::JavaVM;
use jni::objects::GlobalRef;
use jni::objects::JClass;
use jni::objects::JFloatArray;
use jni::objects::JObject;
use jni::objects::JString;
use jni::objects::JValue;
use jni::sys::jdouble;
use jni::sys::jfloat;
use jni::sys::jint;
use jni::sys::jlong;
use leptoskit_native::FontWeight;
use leptoskit_native::Mutation;
use leptoskit_native::MutationList;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::PointerDecision;
use leptoskit_native::PointerKind;
use leptoskit_native::PointerPhase;
use leptoskit_native::PointerRoute;
use leptoskit_native::PointerSample;
use leptoskit_native::SharedTextMeasurer;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Direction;
use leptoskit_yoga::NodeId;
use leptoskit_yoga::Size;

/// Failure to create the Android host boundary.
#[derive(Debug, thiserror::Error)]
pub enum AndroidError {
	/// A JNI operation failed. A pending Java exception remains available to
	/// the host.
	#[error("android JNI operation failed: {0}")]
	Jni(#[from] jni::errors::Error),
}

type Listener = Rc<dyn Fn(NodeId, &NativeEvent)>;
type PointerListener = Rc<dyn Fn(PointerSample) -> PointerDecision>;
type ClockListener = Rc<dyn Fn(u64) -> PointerDecision>;

#[derive(Default)]
struct EventSink {
	listener: RefCell<Option<Listener>>,
	pointer: RefCell<Option<PointerListener>>,
	clock: RefCell<Option<ClockListener>>,
	nodes: RefCell<HashMap<usize, (NodeId, NativeProps)>>,
}

thread_local! {
	static SINKS: RefCell<HashMap<i64, Weak<EventSink>>> = RefCell::default();
}

static NEXT_HOST: AtomicI64 = AtomicI64::new(1);

/// A UI-thread-owned backend rendering every built-in kind as Android Views.
///
/// Native controls retain their committed values after rejected proposals.
/// Callbacks may synchronously update or remove the emitting node. They run
/// outside the backend's borrows. Dropping the backend detaches listeners,
/// stops animations, and releases the host's widget tree.
///
/// This type is deliberately `!Send` and `!Sync`. The text measurer alone may
/// cross threads; it retains only the VM and a global class reference.
pub struct AndroidBackend {
	vm: Arc<JavaVM>,
	host: GlobalRef,
	handle: i64,
	sink: Rc<EventSink>,
	measurer: SharedTextMeasurer,
	_ui_thread: PhantomData<Rc<()>>,
}

impl std::fmt::Debug for AndroidBackend {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("AndroidBackend")
			.field("handle", &self.handle)
			.finish_non_exhaustive()
	}
}

impl AndroidBackend {
	/// Connects a `dev.leptoskit.widgets.WidgetHost` created on the UI thread.
	///
	/// # Errors
	/// Returns a JNI error if the object is invalid or called off the UI
	/// thread.
	pub fn new(env: &mut JNIEnv<'_>, host: &JObject<'_>) -> Result<Self, AndroidError> {
		env.call_method(host, "checkUiThread", "()V", &[])?;
		let vm = Arc::new(env.get_java_vm()?);
		let class = env.get_object_class(host)?;
		let class = env.new_global_ref(class)?;
		let density = env.call_method(host, "getDensity", "()F", &[])?.f()?;
		let host = env.new_global_ref(host)?;
		let sink = Rc::new(EventSink::default());
		let handle = NEXT_HOST.fetch_add(1, Ordering::Relaxed);

		env.call_method(host.as_obj(), "bind", "(J)V", &[JValue::Long(handle)])?;
		SINKS.with(|sinks| sinks.borrow_mut().insert(handle, Rc::downgrade(&sink)));
		let measurer = Arc::new(AndroidTextMeasurer {
			vm: Arc::clone(&vm),
			class,
			density,
		});
		Ok(Self {
			vm,
			host,
			handle,
			sink,
			measurer,
			_ui_thread: PhantomData,
		})
	}

	/// Replaces the UI-thread listener receiving typed native proposals.
	pub fn set_event_listener(&self, listener: impl Fn(NodeId, &NativeEvent) + 'static) {
		*self.sink.listener.borrow_mut() = Some(Rc::new(listener));
	}

	/// Removes the listener without removing the mounted tree.
	pub fn clear_event_listener(&self) {
		self.sink.listener.borrow_mut().take();
	}

	/// Receives raw pointer samples from the root view, in viewport layout
	/// units.
	///
	/// The host calls this before the platform control sees the touch. The
	/// returned route says whether the control should still handle it.
	pub fn set_pointer_listener(
		&self,
		listener: impl Fn(PointerSample) -> PointerDecision + 'static,
	) {
		*self.sink.pointer.borrow_mut() = Some(Rc::new(listener));
	}

	/// Advances gestures that are waiting on the clock, such as a long press.
	pub fn set_gesture_clock(&self, clock: impl Fn(u64) -> PointerDecision + 'static) {
		*self.sink.clock.borrow_mut() = Some(Rc::new(clock));
	}

	/// Drops the pointer and clock listeners. Platform controls keep working.
	pub fn clear_pointer_listener(&self) {
		self.sink.pointer.borrow_mut().take();
		self.sink.clock.borrow_mut().take();
	}

	pub(crate) fn handle(&self) -> i64 {
		self.handle
	}

	/// Returns a local JNI reference to a node's exact Yoga outer view.
	pub fn view<'local>(
		&self,
		env: &mut JNIEnv<'local>,
		id: NodeId,
	) -> Result<JObject<'local>, AndroidError> {
		Ok(env
			.call_method(
				self.host.as_obj(),
				"view",
				"(J)Landroid/view/View;",
				&[JValue::Long(id.index() as i64)],
			)?
			.l()?)
	}

	fn send(&self, mutations: &[Mutation]) {
		let mut env = self
			.vm
			.attach_current_thread()
			.expect("Android VM remains alive");
		env.with_local_frame(8, |env| -> jni::errors::Result<()> {
			let data =
				serde_json::to_string(mutations).expect("validated finite mutations serialize");
			let data = env.new_string(data)?;
			env.call_method(
				self.host.as_obj(),
				"apply",
				"(Ljava/lang/String;)V",
				&[JValue::Object(&data)],
			)?;
			Ok(())
		})
		.expect("valid Android host applies native mutations");
	}

	fn record(&self, mutation: &Mutation) {
		let mut nodes = self.sink.nodes.borrow_mut();

		match mutation {
			Mutation::Create { id, props, .. } | Mutation::UpdateProps { id, props } => {
				nodes.insert(id.index(), (*id, props.clone()));
			}
			Mutation::Delete { id } => {
				nodes.remove(&id.index());
			}
			_ => {}
		}
	}
}

impl Drop for AndroidBackend {
	fn drop(&mut self) {
		SINKS.with(|sinks| {
			sinks.borrow_mut().remove(&self.handle);
		});
		self.sink.listener.borrow_mut().take();

		if let Ok(mut env) = self.vm.attach_current_thread()
			&& let Err(error) = env.call_method(self.host.as_obj(), "close", "()V", &[])
		{
			// Destruction cannot unwind across JNI. Preserve the pending
			// Java exception and expose the failure rather
			// than ignoring it.
			eprintln!("Android widget teardown failed: {error}");
		}
	}
}

impl NativeBackend for AndroidBackend {
	fn rive_visible(&self, id: NodeId) -> bool {
		let result = self.vm.attach_current_thread().and_then(|mut env| {
			env.with_local_frame(4, |env| {
				env.call_method(
					self.host.as_obj(),
					"riveVisible",
					"(J)Z",
					&[JValue::Long(id.index() as i64)],
				)?
				.z()
			})
		});

		match result {
			Ok(visible) => visible,
			Err(error) => {
				eprintln!("Android Rive visibility failed: {error}");
				false
			}
		}
	}

	fn present_rive(&mut self, id: NodeId, frame: &leptoskit_native::RiveFrame<'_>) -> bool {
		let result = self.vm.attach_current_thread().and_then(|mut env| {
			env.with_local_frame(8, |env| {
				// SAFETY: The synchronous host method copies pixels and never
				// retains the buffer. Java receives only a read-only view, and
				// every local JNI reference is released before this borrowed
				// frame can change.
				let buffer = unsafe {
					env.new_direct_byte_buffer(
						frame.bytes().as_ptr().cast_mut(),
						frame.bytes().len(),
					)
				}?;
				let readonly = env
					.call_method(&buffer, "asReadOnlyBuffer", "()Ljava/nio/ByteBuffer;", &[])?
					.l()?;
				env.call_method(
					self.host.as_obj(),
					"presentRive",
					"(JIILjava/nio/ByteBuffer;)Z",
					&[
						JValue::Long(id.index() as i64),
						JValue::Int(frame.width() as i32),
						JValue::Int(frame.height() as i32),
						JValue::Object(&readonly),
					],
				)?
				.z()
			})
		});

		match result {
			Ok(presented) => presented,
			Err(error) => {
				eprintln!("Android Rive upload failed: {error}");
				false
			}
		}
	}

	fn apply(&mut self, mutations: &MutationList) {
		for mutation in mutations.mutations() {
			self.record(mutation);
		}

		self.send(mutations.mutations());
	}

	fn create(&mut self, id: NodeId, kind: NativeKind, props: &NativeProps) {
		let mutation = Mutation::Create {
			id,
			kind,
			props: props.clone(),
		};
		self.record(&mutation);
		self.send(&[mutation]);
	}

	fn update_props(&mut self, id: NodeId, props: &NativeProps) {
		let mutation = Mutation::UpdateProps {
			id,
			props: props.clone(),
		};
		self.record(&mutation);
		self.send(&[mutation]);
	}

	fn update_layout(&mut self, id: NodeId, x: f32, y: f32, width: f32, height: f32) {
		self.send(&[Mutation::UpdateLayout {
			id,
			x,
			y,
			width,
			height,
		}]);
	}

	fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
		self.send(&[Mutation::InsertChild {
			parent,
			child,
			index,
		}]);
	}

	fn remove_child(&mut self, parent: NodeId, child: NodeId) {
		self.send(&[Mutation::RemoveChild { parent, child }]);
	}

	fn delete(&mut self, id: NodeId) {
		let mutation = Mutation::Delete { id };
		self.record(&mutation);
		self.send(&[mutation]);
	}

	fn text_measurer(&self) -> SharedTextMeasurer {
		Arc::clone(&self.measurer)
	}

	fn set_dark_mode(&mut self, dark: bool) {
		self.vm
			.attach_current_thread()
			.expect("Android VM remains alive")
			.call_method(
				self.host.as_obj(),
				"setDarkMode",
				"(Z)V",
				&[JValue::Bool(u8::from(dark))],
			)
			.expect("valid Android host sets appearance");
	}
}

struct AndroidTextMeasurer {
	vm: Arc<JavaVM>,
	class: GlobalRef,
	density: f32,
}

impl TextMeasurer for AndroidTextMeasurer {
	fn measure(&self, text: &str, style: TextStyle, available: Size<AvailableSpace>) -> Size<f32> {
		let mut env = self
			.vm
			.attach_current_thread()
			.expect("Android VM remains alive");
		env.with_local_frame(8, |env| -> jni::errors::Result<Size<f32>> {
			let text = env.new_string(text)?;
			let width = match available.width {
				AvailableSpace::Definite(w) => w.max(0.0),
				AvailableSpace::MinContent => -2.0,
				AvailableSpace::MaxContent => -1.0,
			};
			let weight = match style.font_weight {
				FontWeight::Regular => 400,
				FontWeight::Medium => 500,
				FontWeight::Semibold => 600,
				FontWeight::Bold => 700,
			};

			let class: &JClass<'_> = self.class.as_obj().into();
			let array = env
				.call_static_method(
					class,
					"measureText",
					"(Ljava/lang/String;FIFFZ)[F",
					&[
						JValue::Object(&text),
						JValue::Float(style.font_size),
						JValue::Int(weight),
						JValue::Float(width),
						JValue::Float(self.density),
						JValue::Bool(u8::from(style.direction == Direction::Rtl)),
					],
				)?
				.l()?;
			let mut size = [0.0; 2];
			env.get_float_array_region(JFloatArray::from(array), 0, &mut size)?;
			Ok(Size::new(size[0], size[1]))
		})
		.expect("Android TextPaint and StaticLayout measure text")
	}
}

// SAFETY: Android calls this symbol using the matching Kotlin external
// declaration. References are scoped to this invocation and never escape it.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_widgets_WidgetHost_nativeEvent(
	mut env: JNIEnv<'_>,
	_class: JClass<'_>,
	handle: jlong,
	index: jlong,
	kind: jint,
	value: jdouble,
	text: JString<'_>,
) {
	let sink = SINKS.with(|sinks| sinks.borrow().get(&handle).and_then(Weak::upgrade));
	let Some(sink) = sink else {
		return;
	};
	let Some((id, props)) = sink.nodes.borrow().get(&(index as usize)).cloned() else {
		return;
	};

	let event = match kind {
		0 => NativeEvent::Press,
		1 | 2 => {
			let value = match env.get_string(&text) {
				Ok(text) => text.into(),
				Err(error) => {
					eprintln!("Android input event failed: {error}");
					return;
				}
			};
			if kind == 1 {
				NativeEvent::Change { value }
			} else {
				NativeEvent::Submit { value }
			}
		}
		3 => NativeEvent::Focus,
		4 => NativeEvent::Blur,
		5 => {
			NativeEvent::Toggle {
				checked: value != 0.0,
			}
		}
		6 => {
			let Some(slider) = props.slider else {
				return;
			};
			let proposal = slider.min() + value.clamp(0.0, 1.0) * (slider.max() - slider.min());
			let Some(value) = slider.proposed_value(proposal) else {
				return;
			};
			NativeEvent::ValueChange { value }
		}
		7 => {
			let offsets: String = match env.get_string(&text) {
				Ok(text) => text.into(),
				Err(_) => return,
			};

			let Ok([offset_x, offset_y]) = serde_json::from_str::<[f32; 2]>(&offsets) else {
				return;
			};
			NativeEvent::Scroll { offset_x, offset_y }
		}
		8 | 9 => {
			let Some(slider) = props.slider else {
				return;
			};
			let increment = slider
				.step()
				.unwrap_or((slider.max() - slider.min()) / 100.0);
			let proposal = if kind == 8 {
				slider.value() + increment
			} else {
				slider.value() - increment
			};

			let Some(value) = slider.proposed_value(proposal) else {
				return;
			};
			NativeEvent::ValueChange { value }
		}
		10 => NativeEvent::PressIn,
		11 => NativeEvent::PressOut,
		_ => return,
	};

	let listener = sink.listener.borrow().clone();

	if let Some(listener) = listener {
		listener(id, &event);
	}
}

/// Routes one pointer sample to the host's listener. A missing host, a missing
/// listener, or a sample the arena does not understand returns platform
/// handling, so the Android control still receives the touch.
pub(crate) fn route_pointer(
	handle: i64,
	pointer: i64,
	phase: i32,
	x: f32,
	y: f32,
	time_ms: i64,
	kind: i32,
) -> i64 {
	let Some(sink) = sink(handle) else {
		return 0;
	};
	let Some(listener) = sink.pointer.borrow().clone() else {
		return 0;
	};
	let Some(sample) = pointer_sample(pointer, phase, x, y, time_ms, kind) else {
		return 0;
	};
	pack(&listener(sample), sample.time_ms)
}

/// Advances the arena clock for recognizers waiting on a deadline.
pub(crate) fn route_clock(handle: i64, time_ms: i64) -> i64 {
	let Some(sink) = sink(handle) else {
		return 0;
	};
	let Some(clock) = sink.clock.borrow().clone() else {
		return 0;
	};
	let now = u64::try_from(time_ms).unwrap_or(0);
	pack(&clock(now), now)
}

fn sink(handle: i64) -> Option<Rc<EventSink>> {
	SINKS.with(|sinks| sinks.borrow().get(&handle).and_then(Weak::upgrade))
}

fn pointer_sample(
	pointer: i64,
	phase: i32,
	x: f32,
	y: f32,
	time_ms: i64,
	kind: i32,
) -> Option<PointerSample> {
	let phase = match phase {
		0 => PointerPhase::Down,
		1 => PointerPhase::Move,
		2 => PointerPhase::Up,
		3 => PointerPhase::Cancel,
		_ => return None,
	};
	let kind = match kind {
		0 => PointerKind::Touch,
		1 => PointerKind::Mouse,
		2 => PointerKind::Pen,
		_ => return None,
	};
	let id = u64::try_from(pointer).ok()?;
	let time_ms = u64::try_from(time_ms).unwrap_or(0);
	Some(PointerSample::new(id, phase, x, y, time_ms).kind(kind))
}

fn pack(decision: &PointerDecision, now: u64) -> i64 {
	let route = match decision.route {
		PointerRoute::Platform => 0,
		PointerRoute::Track => 1,
		PointerRoute::Claim => 2,
	};
	let wait = decision
		.deadline_ms
		.map(|at| at.saturating_sub(now).min(10_000))
		.unwrap_or(0);
	route | i64::try_from(wait).unwrap_or(0) << 8
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_widgets_WidgetHost_nativePointer(
	_env: JNIEnv<'_>,
	_class: JClass<'_>,
	handle: jlong,
	pointer: jlong,
	phase: jint,
	x: jfloat,
	y: jfloat,
	time_ms: jlong,
	kind: jint,
) -> jlong {
	route_pointer(handle, pointer, phase, x, y, time_ms, kind)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_widgets_WidgetHost_nativeGestureAdvance(
	_env: JNIEnv<'_>,
	_class: JClass<'_>,
	handle: jlong,
	time_ms: jlong,
) -> jlong {
	route_clock(handle, time_ms)
}
