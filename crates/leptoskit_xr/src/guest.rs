//! The guest's client: a Wasm guest asks the host for sessions and frames
//! through the `xr` plugin.

use leptoskit_bridge::Error;
use leptoskit_bridge::guest::Client;

use crate::math::Pose;
use crate::protocol::*;
use crate::types::*;
use crate::wire::Wire;
use crate::wire::decode;
use crate::wire::encode;

async fn call<T: Wire>(
	client: Client,
	method: leptoskit_bridge::MethodId,
	request: &impl Wire,
) -> Result<T, Error> {
	decode(&client.call(method, &encode(request))?.await?)
}

/// The resolved `xr` plugin. Make it once and copy it around.
#[derive(Debug, Clone, Copy)]
pub struct Xr(Client);

impl Xr {
	/// Resolves the plugin, or fails with `Unavailable` when the host has
	/// none.
	pub fn new() -> Result<Self, Error> {
		Client::new(NAME).map(Self)
	}

	/// What the device can do, mode by mode.
	pub async fn capabilities(self) -> Result<Capabilities, Error> {
		call(self.0, CAPABILITIES, &()).await
	}

	/// Whether `mode` can start now. With `install`, opens the store when the
	/// platform's AR service is missing; only ask after the user chose AR.
	pub async fn prepare(self, mode: Mode, install: bool) -> Result<Availability, Error> {
		call(self.0, PREPARE, &Prepare { mode, install }).await
	}

	/// Starts a session. Only one runs at a time.
	pub async fn start(self, config: SessionConfig) -> Result<Session, Error> {
		let info = call(self.0, START, &config).await?;

		Ok(Session {
			client: self.0,
			info,
			after: 0,
			planes: PlaneCache::default(),
		})
	}
}

/// A running session. Dropping it doesn't stop the host's session; call
/// [`Session::stop`].
#[derive(Debug)]
pub struct Session {
	client: Client,
	info: SessionInfo,
	after: u64,
	planes: PlaneCache,
}

impl Session {
	/// The mode, features, and reference space the host granted.
	pub fn info(&self) -> SessionInfo {
		self.info
	}

	/// Waits for the next frame, which may skip frames the host tracked while
	/// the guest was busy. Fails with `Cancelled` once the session stops.
	pub async fn next_frame(&mut self) -> Result<Frame, Error> {
		let request = NextFrame {
			after: self.after,
			planes_revision: self.planes.revision(),
		};
		let mut frame: Frame = call(self.client, NEXT_FRAME, &request).await?;
		self.after = frame.sequence;
		self.planes.merge(&mut frame);
		Ok(frame)
	}

	/// Changes the features, reference space, or frame rate.
	pub async fn configure(&mut self, config: SessionConfig) -> Result<SessionInfo, Error> {
		self.info = call(self.client, CONFIGURE, &config).await?;
		Ok(self.info)
	}

	/// Casts a ray against the real world, nearest hit first.
	pub async fn hit_test(&self, request: HitTestRequest) -> Result<Vec<Hit>, Error> {
		call(self.client, HIT_TEST, &request).await
	}

	/// Pins a point to the real world, optionally to a plane, and returns its
	/// id. Its pose arrives in each frame's `anchors`.
	pub async fn add_anchor(&self, pose: Pose, plane: Option<u64>) -> Result<u64, Error> {
		call(self.client, ADD_ANCHOR, &AddAnchor { pose, plane }).await
	}

	/// Releases an anchor.
	pub async fn remove_anchor(&self, id: u64) -> Result<(), Error> {
		call(self.client, REMOVE_ANCHOR, &id).await
	}

	/// Ends the session and releases the camera.
	pub async fn stop(self) -> Result<(), Error> {
		call(self.client, STOP, &()).await
	}
}
