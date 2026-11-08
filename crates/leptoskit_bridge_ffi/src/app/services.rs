//! Executor-neutral services retain task state independently of the
//! interpreter.

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;

use leptoskit_bridge::CallId;
use leptoskit_bridge::Error;
use leptoskit_bridge::Request;
use leptoskit_native::HeadlessBackend;
use leptoskit_wasm::WasmHost;

#[cfg(test)]
mod ffi_tests;
#[cfg(test)]
mod tests;

mod ffi;
pub use ffi::*;

/// Owned work for the existing immediate, worker and platform plugin router.
#[derive(Default)]
pub struct ServiceWork {
	/// Requests not handled by this service, in their original order.
	pub requests: Vec<Request>,
	/// Cancellation notifications not handled by this service.
	pub cancelled: Vec<CallId>,
}

/// Teardown owns task state, and never borrows the application or interpreter.
/// Ready, including an error, means all service-owned work has actually joined.
pub type ServiceClose = Pin<Box<dyn Future<Output = Result<(), Error>>>>;

/// Why the host is polling the service right now.
///
/// A background tick comes from the platform's background machinery (an Android
/// foreground service interval or an iOS background task). An app call is any
/// ordinary guest entry point. The host sets this before each poll, and it is
/// consumed after the poll returns, so a service that wants the value must
/// retain it in [`AppService::set_wake_reason`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ServiceWakeReason {
	/// An ordinary guest entry point or the service's own async completion.
	#[default]
	App,
	/// A platform background tick.
	Background,
}
impl ServiceWakeReason {
	/// Decodes the reason from the FFI boundary: 0 is [`Self::App`], 1 is
	/// [`Self::Background`].
	pub fn decode(value: i32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::App),
			1 => Ok(Self::Background),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// One application-selected service may register several explicit capabilities.
///
/// Poll only borrows the actual interpreter synchronously. Native I/O futures
/// retain owned state. Register the supplied waker when waiting for I/O and
/// return every unrelated request and cancellation intact. Host configuration
/// owns endpoints, credentials, grants and resource limits.
pub trait AppService {
	/// Admits work and delivers ready completions on the interpreter's owner.
	fn poll(
		&mut self,
		host: &mut WasmHost<HeadlessBackend>,
		context: &mut Context<'_>,
	) -> Result<ServiceWork, Error>;

	/// Receives the reason for the imminent [`Self::poll`]. The default ignores
	/// it; a service that distinguishes a background tick from an app call
	/// overrides this to retain the value until the next poll.
	fn set_wake_reason(&mut self, _reason: ServiceWakeReason) {}

	/// Stops admission and invalidates delivery synchronously, then returns the
	/// owned future that cancels and joins all service work.
	fn close(self: Box<Self>) -> ServiceClose;
}

/// Runs before guest start, after wake and close ownership have been installed.
/// An error must not leave work running outside a returned service owner.
pub type ServiceInstaller =
	Box<dyn FnOnce(&mut WasmHost<HeadlessBackend>) -> Result<Box<dyn AppService>, Error>>;

/// Retains and polls closing owners on the same UI thread until they complete.
/// This scheduler must remain usable during initialization failure and Drop.
pub type CloseScheduler = Rc<dyn Fn(ClosingApp)>;

struct Signal {
	active: AtomicBool,
	queued: AtomicBool,
	notify: Box<dyn Fn() + Send + Sync>,
}

impl Wake for Signal {
	fn wake(self: Arc<Self>) {
		self.wake_by_ref();
	}

	fn wake_by_ref(self: &Arc<Self>) {
		if self.active.load(Ordering::Acquire) && !self.queued.swap(true, Ordering::AcqRel) {
			(self.notify)();
		}
	}
}

/// A coalesced thread-safe notification with no application pointer or borrow.
/// The callback enqueues a UI turn and must not reenter the interpreter.
/// Already enqueued notifications may arrive after close; their platform owner
/// must check its current application before polling. The callback's captured
/// state remains owned until every concurrent wake has returned.
#[derive(Clone)]
pub struct ServiceWake(Arc<Signal>);

impl ServiceWake {
	/// Captures only independently owned notification state.
	pub fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
		Self(Arc::new(Signal {
			active: AtomicBool::new(true),
			queued: AtomicBool::new(false),
			notify: Box::new(notify),
		}))
	}

	fn waker(&self) -> Waker {
		self.0.queued.store(false, Ordering::Release);
		Waker::from(Arc::clone(&self.0))
	}

	/// Schedules one coalesced platform turn from any thread.
	pub(super) fn notify(&self) {
		self.0.wake_by_ref();
	}

	fn invalidate(&self) {
		self.0.active.store(false, Ordering::Release);
	}

	pub(super) fn fork(&self) -> Self {
		let signal = Arc::clone(&self.0);
		Self::new(move || (signal.notify)())
	}
}

pub(super) enum ServiceState {
	Deferred(ServiceInstaller),
	Failed,
	Active {
		service: Box<dyn AppService>,
		wake: ServiceWake,
	},
}

impl ServiceState {
	pub(super) fn poll(
		&mut self,
		host: &mut WasmHost<HeadlessBackend>,
		reason: ServiceWakeReason,
	) -> Result<ServiceWork, Error> {
		let Self::Active { service, wake } = self else {
			return Err(Error::Unavailable);
		};
		service.set_wake_reason(reason);
		let waker = wake.waker();
		service.poll(host, &mut Context::from_waker(&waker))
	}

	pub(super) fn close(self) -> Option<ServiceClose> {
		let Self::Active { service, wake } = self else {
			return None;
		};
		wake.invalidate();
		Some(service.close())
	}
}

/// Owns the actual interpreter until service teardown has joined.
/// Cancelling this future transfers its remainder to the installed scheduler.
/// The scheduler, rather than Drop, performs asynchronous polling.
#[must_use = "await close or retain it in the application's close scheduler"]
pub struct ClosingApp {
	host: Option<WasmHost<HeadlessBackend>>,
	future: Option<ServiceClose>,
	schedule: Option<CloseScheduler>,
	wake: Option<ServiceWake>,
	result: Option<Result<(), Error>>,
}

impl ClosingApp {
	pub(super) fn new(
		host: Option<WasmHost<HeadlessBackend>>,
		service: Option<ServiceState>,
		schedule: Option<CloseScheduler>,
		wake: Option<ServiceWake>,
	) -> Self {
		Self {
			host,
			future: service.and_then(ServiceState::close),
			schedule,
			wake,
			result: None,
		}
	}
}

impl Future for ClosingApp {
	type Output = Result<(), Error>;

	fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
		if let Some(result) = self.result {
			return Poll::Ready(result);
		}

		let result = match self.future.as_mut() {
			Some(future) => std::task::ready!(future.as_mut().poll(context)),
			None => Ok(()),
		};

		self.future.take();
		self.result = Some(result);

		if let Some(wake) = &self.wake {
			wake.invalidate();
		}

		self.host.take();
		Poll::Ready(result)
	}
}

impl Drop for ClosingApp {
	fn drop(&mut self) {
		if self.future.is_none() {
			return;
		}

		let schedule = self
			.schedule
			.take()
			.expect("activated services own a close scheduler");
		let retained = Self {
			host: self.host.take(),
			future: self.future.take(),
			schedule: Some(Rc::clone(&schedule)),
			wake: self.wake.take(),
			result: self.result,
		};
		schedule(retained);
	}
}
