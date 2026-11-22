//! Cached native home-widgets client.
use super::*;

/// Resolves the host once, then uses numeric method identifiers.
#[derive(Debug, Clone, Copy)]
pub struct HomeWidgets(leptoskit_bridge::guest::Client);

impl HomeWidgets {
	/// Resolves an explicitly installed host plugin.
	pub fn new() -> Result<Self, Error> {
		leptoskit_bridge::guest::Client::new(NAME).map(Self)
	}

	/// Reports what this host can do without touching widget storage.
	pub async fn capability(self) -> Result<Capability, Error> {
		Capability::decode(&self.0.call(CAPABILITY, &[])?.await?)
	}

	/// Publishes a snapshot for the widget extension to read. Acceptance is
	/// not proof that a home-screen instance exists or refreshed.
	pub async fn set(self, snapshot: &Snapshot) -> Result<(), Error> {
		self.empty(SET, &snapshot.encode()?).await
	}

	/// Reads the snapshot previously published for `kind`.
	pub async fn get(self, kind: &Kind) -> Result<Snapshot, Error> {
		Snapshot::decode(&self.0.call(GET, &kind.encode()?)?.await?)
	}

	/// Removes the published snapshot for `kind`.
	pub async fn clear(self, kind: &Kind) -> Result<(), Error> {
		self.empty(CLEAR, &kind.encode()?).await
	}

	/// Asks the OS to reload widget timelines. `None` reloads every kind this
	/// plugin owns. Reload does not create or pin a widget.
	pub async fn reload(self, kind: Option<&Kind>) -> Result<(), Error> {
		self.empty(RELOAD, &encode_reload(kind)?).await
	}

	/// Reports whether any instance of `kind` is pinned, when the platform can
	/// tell. `WidgetKit` returns [`Installed::Unknown`].
	pub async fn installed(self, kind: &Kind) -> Result<Installed, Error> {
		Installed::decode(&self.0.call(INSTALLED, &kind.encode()?)?.await?)
	}

	/// Publishes a multi-entry [`TimelineSchedule`] for `WidgetKit` or Android
	/// App Widgets.
	pub async fn set_timeline(self, schedule: &TimelineSchedule) -> Result<(), Error> {
		self.empty(SET_TIMELINE, &schedule.encode()?).await
	}

	/// Reads the [`TimelineSchedule`] previously published for `kind`.
	pub async fn get_timeline(self, kind: &Kind) -> Result<TimelineSchedule, Error> {
		TimelineSchedule::decode(&self.0.call(GET_TIMELINE, &kind.encode()?)?.await?)
	}

	/// Publishes state for an iOS 18+ Control Center `ControlWidget`.
	pub async fn set_control(self, control: &ControlCenterControl) -> Result<(), Error> {
		self.empty(SET_CONTROL, &control.encode()?).await
	}

	/// Reads the [`ControlCenterControl`] previously stored for `id`.
	pub async fn get_control(self, id: &Kind) -> Result<ControlCenterControl, Error> {
		ControlCenterControl::decode(&self.0.call(GET_CONTROL, &id.encode()?)?.await?)
	}

	/// Asks iOS 18+ `ControlCenter` to reload controls (`None` reloads all).
	pub async fn reload_controls(self, kind: Option<&Kind>) -> Result<(), Error> {
		self.empty(RELOAD_CONTROLS, &encode_reload(kind)?).await
	}

	/// Publishes state for an Android Quick Settings `TileService`.
	pub async fn set_quick_tile(self, tile: &QuickSettingsTile) -> Result<(), Error> {
		self.empty(SET_QUICK_TILE, &tile.encode()?).await
	}

	/// Reads the [`QuickSettingsTile`] previously stored for `id`.
	pub async fn get_quick_tile(self, id: &Kind) -> Result<QuickSettingsTile, Error> {
		QuickSettingsTile::decode(&self.0.call(GET_QUICK_TILE, &id.encode()?)?.await?)
	}

	async fn empty(self, method: MethodId, payload: &[u8]) -> Result<(), Error> {
		if self.0.call(method, payload)?.await?.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}
}
