//! Cached native notification client.
use super::*;

/// Resolves the host once, then uses numeric method identifiers.
#[derive(Debug, Clone, Copy)]
pub struct Notifications(leptoskit_bridge::guest::Client);
impl Notifications {
	/// Resolves an explicitly installed host plugin.
	pub fn new() -> Result<Self, Error> {
		leptoskit_bridge::guest::Client::new(NAME).map(Self)
	}

	/// Queries settings without prompting.
	pub async fn permission(self) -> Result<Permission, Error> {
		Permission::decode(&self.0.call(PERMISSION, &[])?.await?)
	}

	/// Requests OS permission. The OS dialog itself may outlive cancellation.
	pub async fn request_permission(self) -> Result<Permission, Error> {
		Permission::decode(&self.0.call(REQUEST_PERMISSION, &[])?.await?)
	}

	/// Submits immediate local content. Acceptance is not proof of visible
	/// delivery.
	pub async fn show(self, notification: &Notification) -> Result<(), Error> {
		self.empty(SHOW, &notification.encode()?).await
	}

	/// Removes the named notification, including pending native work.
	pub async fn cancel(self, id: &Identifier) -> Result<(), Error> {
		self.empty(CANCEL, &id.encode()?).await
	}

	/// Removes this plugin's notifications without touching other app
	/// providers.
	pub async fn cancel_all(self) -> Result<(), Error> {
		self.empty(CANCEL_ALL, &[]).await
	}

	/// Creates/updates Android channel metadata; does not override user
	/// settings.
	pub async fn configure_channel(self, channel: &Channel) -> Result<(), Error> {
		self.empty(CONFIGURE_CHANNEL, &channel.encode()?).await
	}

	/// Deletes an Android channel. Other platforms report Unavailable.
	pub async fn delete_channel(self, id: &Identifier) -> Result<(), Error> {
		self.empty(DELETE_CHANNEL, &id.encode()?).await
	}

	/// Reads the host's latest remote-registration answer without side
	/// effects. iOS, macOS, and Android; every other platform answers
	/// Unavailable. Safe at launch: reading never prompts and never
	/// registers.
	pub async fn remote_snapshot(self) -> Result<RemoteRegistration, Error> {
		RemoteRegistration::decode(&self.0.call(REMOTE_TOKEN, &[])?.await?)
	}

	/// Asks the OS to register for remote notifications and waits, bounded
	/// by the host, for its answer, returned as the snapshot it produced.
	/// iOS, macOS, and Android; every other platform answers Unavailable.
	/// When the host's wait bound passes with no new answer, the reply is
	/// [`RemoteRegistration::unconfirmed`]: the broker's previous answer —
	/// a token an earlier registration issued — is never handed back as if
	/// it were fresh; read it separately with
	/// [`Notifications::remote_snapshot`].
	pub async fn register_remote(self) -> Result<RemoteRegistration, Error> {
		RemoteRegistration::decode(&self.0.call(REMOTE_REGISTER, &[])?.await?)
	}

	/// Reads what the device says about itself — preferred locale, time
	/// zone, app version — without side effects. iOS, macOS, and Android;
	/// every other platform, and a host that predates the method, answers
	/// Unavailable.
	pub async fn device_info(self) -> Result<DeviceInfo, Error> {
		DeviceInfo::decode(&self.0.call(DEVICE, &[])?.await?)
	}

	/// Reads which APNs environment the running build is signed for,
	/// without side effects. iOS and macOS; every other platform, and a
	/// host that predates the method, answers Unavailable.
	pub async fn aps_environment(self) -> Result<ApsEnvironment, Error> {
		ApsEnvironment::decode(&self.0.call(APS_ENVIRONMENT, &[])?.await?)
	}

	/// Takes what the host has received since the last call — taps,
	/// notifications that arrived while the app was open, and silent data
	/// messages — oldest first. With nothing queued the host waits up to
	/// `wait_millis` (capped at [`MAX_RECEIVED_WAIT_MILLIS`]) and then
	/// answers an empty batch; `0` never waits. Dropping the future
	/// cancels the wait and loses nothing: what arrives stays queued for
	/// the next call. iOS, macOS, and Android; every other platform, and a
	/// host that predates the method, answers Unavailable.
	pub async fn received(self, wait_millis: u32) -> Result<Vec<ReceivedItem>, Error> {
		ReceivedItem::decode_batch(
			&self
				.0
				.call(RECEIVED, &encode_received_wait(wait_millis))?
				.await?,
		)
	}

	async fn empty(self, method: MethodId, payload: &[u8]) -> Result<(), Error> {
		if self.0.call(method, payload)?.await?.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}
}
