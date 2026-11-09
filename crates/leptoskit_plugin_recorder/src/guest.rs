//! Cached native recorder client.
use leptoskit_bridge::Error;

use super::*;

/// Resolves the host once, then uses numeric method identifiers.
#[derive(Debug, Clone, Copy)]
pub struct Recorder(leptoskit_bridge::guest::Client);
impl Recorder {
	/// Resolves an explicitly installed host plugin.
	pub fn new() -> Result<Self, Error> {
		leptoskit_bridge::guest::Client::new(NAME).map(Self)
	}

	/// Reports the microphone permission state without prompting.
	pub async fn permission(self) -> Result<Permission, Error> {
		Permission::decode(&self.0.call(PERMISSION, &[])?.await?)
	}

	/// Shows the system's microphone permission screen and reports the state
	/// it left behind. Cancelling the call cannot dismiss the screen.
	pub async fn request_permission(self) -> Result<Permission, Error> {
		Permission::decode(&self.0.call(REQUEST_PERMISSION, &[])?.await?)
	}

	/// Starts continuous recording. Any segments an earlier session left
	/// behind are re-queued first, and the reply reports what that recovery
	/// found. On Android the app must be in the foreground or run the
	/// `background` plugin's microphone-typed foreground service; on iOS
	/// recording can only start in the foreground.
	pub async fn start(self, options: &Options) -> Result<Recovery, Error> {
		Recovery::decode(&self.0.call(START, &options.encode()?)?.await?)
	}

	/// Stops recording and finishes the current segment so it is delivered
	/// by [`Self::next_segment`], never lost.
	pub async fn stop(self) -> Result<(), Error> {
		self.empty(STOP).await
	}

	/// Reports the current recording state.
	pub async fn status(self) -> Result<Status, Error> {
		Status::decode(&self.0.call(STATUS, &[])?.await?)
	}

	/// Waits for the next finished segment, rotating on the configured
	/// boundary. The wait is cancellable: dropping the future abandons it
	/// without losing audio. A `None` reply means no segment is waiting and
	/// nothing is recording, so the stream has ended.
	pub async fn next_segment(self) -> Result<Option<Segment>, Error> {
		Segment::decode(&self.0.call(NEXT_SEGMENT, &[])?.await?)
	}

	/// Delivers any segments an earlier session left behind, without
	/// starting to record. Only valid while idle.
	pub async fn recover(self) -> Result<Recovery, Error> {
		Recovery::decode(&self.0.call(RECOVER, &[])?.await?)
	}

	async fn empty(self, method: MethodId) -> Result<(), Error> {
		if self.0.call(method, &[])?.await?.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}
}
