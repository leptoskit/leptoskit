//! <!-- {=p_healthSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_health` as a direct dependency and select the `health`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Windows, Linux, and terminal hosts currently omit
//! runtime plugins; use explicit registration in a custom host there. Guests
//! enable the `guest` feature and reuse `guest::Health` rather than resolving
//! the plugin on every call. Android reads Health Connect and iOS reads
//! `HealthKit`; the app supplies the Apple purpose string and any background or
//! history manifest permissions it wants to request.
//!
//! <!-- {/p_healthSummary} -->
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

mod records;
mod types;
mod wire;

pub use records::*;
pub use types::*;

/// Automatic native registration name.
pub const NAME: &str = "health";
/// Whether the health store exists, empty request; replies with
/// [`Availability`].
pub const AVAILABILITY: MethodId = MethodId(0);
/// The data types and extra permissions the store supports, empty request.
pub const FEATURES: MethodId = MethodId(1);
/// Current grants, empty request; never prompts.
pub const GRANTED: MethodId = MethodId(2);
/// Shows the permission screen: a type mask and [`Extra`] flags.
pub const REQUEST: MethodId = MethodId(3);
/// One page of samples, as [`read_request`] encodes it.
pub const READ: MethodId = MethodId(4);
/// A token for later changes: a type mask and zero flags.
pub const CHANGES_TOKEN: MethodId = MethodId(5);
/// The changes after a token, as [`changes_request`] encodes it.
pub const CHANGES: MethodId = MethodId(6);
/// Complete native method table.
pub const METHODS: &[MethodId] = &[
	AVAILABILITY,
	FEATURES,
	GRANTED,
	REQUEST,
	READ,
	CHANGES_TOKEN,
	CHANGES,
];

/// Native guest client. Requests never start until polled.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Cached bridge handle.
	#[derive(Debug, Clone, Copy)]
	pub struct Health(leptoskit_bridge::guest::Client);

	impl Health {
		/// Resolves the selected plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Whether this device has a health store the app can use.
		pub async fn availability(self) -> Result<Availability, Error> {
			Availability::decode(&self.0.call(AVAILABILITY, &[])?.await?)
		}

		/// Which data types and extra permissions the store supports.
		/// Fails with `Unavailable` unless [`Self::availability`] is
		/// `Available`.
		pub async fn features(self) -> Result<Features, Error> {
			Features::decode(&self.0.call(FEATURES, &[])?.await?)
		}

		/// What the user has allowed, without asking.
		pub async fn granted_permissions(self) -> Result<Grants, Error> {
			Grants::decode(&self.0.call(GRANTED, &[])?.await?)
		}

		/// Shows the platform's permission screen for `types`, plus the
		/// Android permissions in `extra`, and returns what is allowed
		/// afterwards. Types this platform doesn't record are left out.
		/// Cancelling the call cannot dismiss the screen.
		pub async fn request_permissions(
			self,
			types: &[DataType],
			extra: Extra,
		) -> Result<Grants, Error> {
			let request = types_request(DataTypes::of(types), extra.bits())?;
			Grants::decode(&self.0.call(REQUEST, &request)?.await?)
		}

		/// One page of `data_type` samples overlapping `start_ms..end_ms`
		/// (Unix milliseconds), oldest first. `page_size` is 1 to
		/// [`MAX_PAGE_SIZE`]; pass the previous page's `next_page_token` for
		/// the next. A page too large for the bridge fails with `TooLarge`:
		/// ask for fewer samples.
		pub async fn read(
			self,
			data_type: DataType,
			start_ms: i64,
			end_ms: i64,
			page_size: u32,
			page_token: Option<&str>,
		) -> Result<Page, Error> {
			let request = read_request(data_type, start_ms, end_ms, page_size, page_token)?;
			Page::decode(&self.0.call(READ, &request)?.await?)
		}

		/// A token that [`Self::changes`] reports changes after, for `types`.
		pub async fn changes_token(self, types: &[DataType]) -> Result<ChangesToken, Error> {
			let request = types_request(DataTypes::of(types), 0)?;
			ChangesToken::decode(&self.0.call(CHANGES_TOKEN, &request)?.await?)
		}

		/// The samples written and deleted after `token`. Keep calling with
		/// `next_token` while `has_more` is true.
		pub async fn changes(self, token: &ChangesToken) -> Result<Changes, Error> {
			let request = changes_request(token)?;
			Changes::decode(&self.0.call(CHANGES, &request)?.await?)
		}
	}
}

/// Browsers have no health store.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always [`Availability::Unsupported`].
	pub fn availability() -> Availability {
		Availability::Unsupported
	}

	/// Always fails with `Unavailable`.
	pub fn read(
		_: DataType,
		_: i64,
		_: i64,
		_: u32,
		_: Option<&str>,
	) -> std::future::Ready<Result<Page, Error>> {
		std::future::ready(Err(Error::Unavailable))
	}
}

/// Desktops have no health store.
#[cfg(not(target_arch = "wasm32"))]
pub mod desktop {
	use super::*;

	/// Always [`Availability::Unsupported`].
	pub fn availability() -> Availability {
		Availability::Unsupported
	}

	/// Always fails with `Unavailable`.
	pub fn read(
		_: DataType,
		_: i64,
		_: i64,
		_: u32,
		_: Option<&str>,
	) -> std::future::Ready<Result<Page, Error>> {
		std::future::ready(Err(Error::Unavailable))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn methods_are_dense_and_in_order() {
		for (index, method) in METHODS.iter().enumerate() {
			assert_eq!(method.0 as usize, index);
		}
	}

	#[cfg(not(target_arch = "wasm32"))]
	#[test]
	fn desktop_reports_no_health_store() {
		assert_eq!(desktop::availability(), Availability::Unsupported);
		assert_eq!(
			desktop::read(DataType::Steps, 0, 1, 1, None).into_inner(),
			Err(Error::Unavailable)
		);
	}
}
