//! What a page asks the server for: the operations, their answers, and the
//! reasons the server names when it turns one down.
//!
//! The browser calls these by type. The service runs their bodies, and each
//! body hands the request to the service's own [`Backend`], which the
//! service builds for that one request from its pass, its session, and where
//! it came from. Nothing here opens a database or reads a cookie.

#![expect(
	unused_qualifications,
	reason = "the operation macros name `leptoskit::rpc` in full beside this module's import"
)]

use leptoskit::rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::model::Persona;

/// The refusal code for a day that has handed out all its people.
pub const FULL: &str = "full";

/// Where a visitor goes once they have a person to be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Minted {
	/// The local sign-in address for the person, carrying the validated
	/// return target.
	pub next: String,
}

/// The demo day a server is serving.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Day {
	/// The generation's UTC day, `YYYY-MM-DD`.
	pub day: String,
}

impl rpc::Resumable for Day {
	type Cursor = String;

	fn cursor(&self) -> String {
		self.day.clone()
	}
}

/// Gives this browser a person to be, or reuses the one it has, and says
/// where to sign in as them.
///
/// Refusals carry a code: [`rpc::RATE_LIMITED`] with how long to wait when a
/// network has asked for too many new people this hour, and [`FULL`] when
/// the day has handed out all it has.
#[rpc::mutation(crate = "leptoskit::rpc")]
pub async fn mint(
	cx: &rpc::MutationCx,
	persona: Persona,
	return_to: String,
) -> rpc::Result<Minted> {
	cx.require::<Port>()?.0.mint(persona, return_to).await
}

/// The day the server is serving, sent once each time the stream opens.
///
/// A page compares it with the day it was made for. The stream stays open
/// and says nothing more; a page that loses it opens it again, and so hears
/// the next day's server name its own day.
#[rpc::stream(resumable, crate = "leptoskit::rpc")]
pub async fn generation(
	cx: &rpc::StreamCx,
	cursor: Option<String>,
) -> rpc::Result<impl futures::Stream<Item = Day>> {
	// The answer is always today's day, wherever the page last got to.
	let _ = cursor;
	cx.require::<Port>()?.0.generation()
}

/// The service's side of the operations, for one request.
#[cfg(feature = "ssr")]
pub trait Backend: Send + Sync + 'static {
	/// Mints or reuses this request's pass and person.
	fn mint(
		&self,
		persona: Persona,
		return_to: String,
	) -> futures::future::BoxFuture<'_, rpc::Result<Minted>>;

	/// The serving generation's day, then silence for as long as the
	/// generation serves.
	///
	/// # Errors
	///
	/// When the generation has no room for another watcher.
	fn generation(&self) -> rpc::Result<futures::stream::BoxStream<'static, Day>>;
}

/// The request's [`Backend`], as the operations find it in their context.
#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct Port(pub std::sync::Arc<dyn Backend>);
