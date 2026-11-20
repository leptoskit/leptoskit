//! What a page asks the admin's server for while it is open.
//!
//! The browser opens these by type. The admin runs their bodies, and each
//! body hands the request to the admin's own [`Backend`], which the admin
//! builds once it knows the caller is a signed-in admin. Nothing here reads
//! a cookie or the control service.

#![expect(
	unused_qualifications,
	reason = "the operation macros name `leptoskit::rpc` in full beside this module's import"
)]

use leptoskit::rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::components::Tone;
use crate::model::Pulse;
use crate::model::live::Arrival;

/// What happened to an operation, as it happens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Progress {
	/// One step: its place in the operation, what it said, and its phase.
	Step {
		/// Steps count from 1, in the order they happened.
		sequence: u64,
		message: String,
		tone: Tone,
		/// The phase in a word: `Running`.
		word: String,
	},
	/// It finished. The page loads again to show how.
	Done,
}

/// Operation `id`'s steps from here on, then [`Progress::Done`] once it
/// finishes. An operation that already finished says `Done` at once.
#[rpc::stream(crate = "leptoskit::rpc")]
pub async fn operation_progress(
	cx: &rpc::StreamCx,
	id: String,
) -> rpc::Result<impl futures::Stream<Item = Progress>> {
	cx.require::<Port>()?.0.progress(id)
}

/// What the analytics stream says: how fast events arrive, and each event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum Live {
	/// The pulse, about once a second.
	Pulse(Pulse),
	/// One event, as it arrives.
	Event(Arrival),
}

/// The pulse about once a second, and with `events`, every event from now
/// on.
#[rpc::stream(crate = "leptoskit::rpc")]
pub async fn analytics_live(
	cx: &rpc::StreamCx,
	events: bool,
) -> rpc::Result<impl futures::Stream<Item = Live>> {
	cx.require::<Port>()?.0.analytics(events)
}

/// The admin's side of the streams, for one signed-in admin.
#[cfg(feature = "ssr")]
pub trait Backend: Send + Sync + 'static {
	/// What happens to operation `id` from now on.
	///
	/// # Errors
	///
	/// When there is no such operation, or no control service.
	fn progress(&self, id: String) -> rpc::Result<futures::stream::BoxStream<'static, Progress>>;

	/// The analytics pulse, and with `events`, each event.
	///
	/// # Errors
	///
	/// When the admin isn't connected to an analytics service.
	fn analytics(&self, events: bool) -> rpc::Result<futures::stream::BoxStream<'static, Live>>;
}

/// The caller's [`Backend`], as the streams find it in their context.
#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct Port(pub std::sync::Arc<dyn Backend>);
