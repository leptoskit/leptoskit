//! Android system Back (and similar host signals) → router history.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::PoisonError;

use crate::Routable;
use crate::Router;

/// Whether the guest consumed the system Back signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemBack {
	/// The router popped a route; the host should not finish the activity.
	Consumed,
	/// Nothing to pop; the host may leave the app.
	Propagate,
}

type Handler = Arc<dyn Fn() -> SystemBack + Send + Sync>;

fn handler_slot() -> &'static Mutex<Option<Handler>> {
	static SLOT: OnceLock<Mutex<Option<Handler>>> = OnceLock::new();
	SLOT.get_or_init(|| Mutex::new(None))
}

impl<R> Router<R>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	/// Installs this router as the process-wide system-Back handler that
	/// [`dispatch_system_back`] calls. Native apps call this once when they
	/// mount; the last binder wins.
	pub fn bind_system_back(&self) {
		let router = self.clone();
		let handler: Handler = Arc::new(move || router.handle_system_back());
		*handler_slot()
			.lock()
			.unwrap_or_else(PoisonError::into_inner) = Some(handler);
	}

	/// Pops when possible. Used by hosts and by [`dispatch_system_back`].
	pub fn handle_system_back(&self) -> SystemBack {
		if self.back() {
			SystemBack::Consumed
		} else {
			SystemBack::Propagate
		}
	}
}

/// Clears any binder installed by [`Router::bind_system_back`]. Tests call
/// this between cases.
pub fn clear_system_back_handler() {
	*handler_slot()
		.lock()
		.unwrap_or_else(PoisonError::into_inner) = None;
}

/// Invokes the bound handler, or [`SystemBack::Propagate`] when none is set.
///
/// The native guest exports this as `leptoskit_system_back` (returning 1 when
/// consumed).
pub fn dispatch_system_back() -> SystemBack {
	handler_slot()
		.lock()
		.unwrap_or_else(PoisonError::into_inner)
		.as_ref()
		.map_or(SystemBack::Propagate, |handler| handler())
}
