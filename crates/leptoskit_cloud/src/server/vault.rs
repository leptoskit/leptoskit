//! The vault: where secret values are kept, encrypted, and the one way a
//! value comes back out.
//!
//! Envelope encryption with AES-256-GCM-SIV, through `leptoskit_auth`'s
//! [`Keys::seal`]:
//!
//! - each organisation has a **data key**, 32 random bytes made the first time
//!   it stores a secret, kept wrapped by the newest **master key**;
//! - a value is sealed with its organisation's data key, bound to the
//!   organisation, project, environment, name, and version it was stored as. A
//!   row copied onto another secret, another environment, or an older version
//!   of itself doesn't open.
//!
//! The master keys come from the operator, newest first, as
//! `leptoskit_auth`'s keys do. A wrapped data key names the master key that
//! wrapped it, so any listed key opens it after a rotation.
//!
//! Nothing here is `Serialize`, and nothing's `Debug` shows a value or a
//! key.

use leptoskit_auth::Keys;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use uuid::Uuid;

use crate::tables::DataKey;
use crate::tables::Database;
use crate::tables::Environment;
use crate::tables::NewDataKey;
use crate::tables::NotificationTenant;
use crate::tables::Organization;
use crate::tables::Project;
use crate::tables::Secret;

/// What a wrapped data key is bound to, before its organisation.
const DATA_KEY_PURPOSE: &[u8] = b"leptoskit-cloud vault data key v1";

/// What a sealed value is bound to, before its place.
const VALUE_PURPOSE: &[u8] = b"leptoskit-cloud vault secret v1";

/// What a database role's sealed password is bound to, before its place.
const PASSWORD_PURPOSE: &[u8] = b"leptoskit-cloud vault database password v1";

/// What a hosted notifications tenant's sealed values are bound to, before
/// which value and its place.
const TENANT_PURPOSE: &[u8] = b"leptoskit-cloud vault notifications tenant v1";

/// The ID a data key is given inside the [`Keys`] that seals with it. Only
/// one key is ever in it, so the ID says nothing.
const DATA_KEY_ID: &str = "d";

/// The master keys.
#[derive(Clone)]
pub(crate) struct Vault {
	master: Keys,
}

impl std::fmt::Debug for Vault {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Vault")
			.field("master", &self.master.ids())
			.finish()
	}
}

/// Where a secret's value lives: what it is sealed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Place<'a> {
	pub(crate) organization: Id<Organization>,
	pub(crate) project: Id<Project>,
	pub(crate) environment: Id<Environment>,
	/// The secret's own row: a name removed and set again is another row,
	/// so what it held before doesn't open as its first version.
	pub(crate) secret: Id<Secret>,
	pub(crate) name: &'a str,
	pub(crate) version: i64,
}

/// Where a managed database's role's password lives: what it is sealed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DatabasePlace {
	pub(crate) organization: Id<Organization>,
	pub(crate) environment: Id<Environment>,
	pub(crate) database: Id<Database>,
	pub(crate) version: i64,
}

/// Where a hosted notifications tenant's values live: what its role's
/// password and its token are sealed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TenantPlace {
	pub(crate) organization: Id<Organization>,
	pub(crate) environment: Id<Environment>,
	pub(crate) tenant: Id<NotificationTenant>,
	pub(crate) version: i64,
}

/// Which of a tenant's values: neither opens as the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TenantValue {
	Password,
	Token,
}

impl TenantValue {
	const fn label(self) -> &'static [u8] {
		match self {
			Self::Password => b"password",
			Self::Token => b"token",
		}
	}
}

/// An organisation's data key, unwrapped, for one operation. Dropped when
/// the operation ends; never stored or sent.
pub(crate) struct Unwrapped {
	organization: Id<Organization>,
	key: Keys,
}

impl std::fmt::Debug for Unwrapped {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str("Unwrapped([redacted])")
	}
}

/// Whether `error` is the vault saying it couldn't read or write a secret,
/// rather than the database failing.
pub(crate) fn is_unavailable(error: &rpc::Error) -> bool {
	matches!(error, rpc::Error::Server(message) if message.starts_with("vault_unavailable"))
}

pub(crate) fn unavailable() -> rpc::Error {
	// Never the reason: it could name a key, or say which row failed.
	rpc::Error::server("vault_unavailable: the vault couldn't read or write a secret")
}

fn uuid_bytes(id: Uuid) -> [u8; 16] {
	*id.as_bytes()
}

impl Vault {
	pub(crate) fn new(master: Keys) -> Self {
		Self { master }
	}

	/// The organisation's data key, made now when it has none.
	///
	/// A second writer making one at the same moment loses on the table's
	/// unique organisation and is told to try again, when it finds the
	/// first's.
	///
	/// # Errors
	///
	/// `Conflict` (`vault_busy`) for the loser of that race; `Server`
	/// (`vault_unavailable`) when the stored key doesn't open under any
	/// master key, or there is no randomness.
	pub(crate) async fn data_key_for_writing(
		&self,
		tx: &impl leptoskit_db::Connection,
		organization: Id<Organization>,
	) -> rpc::Result<Unwrapped> {
		if let Some(unwrapped) = self.data_key(tx, organization).await? {
			return Ok(unwrapped);
		}

		let mut key = [0_u8; 32];
		getrandom::fill(&mut key).map_err(|_| unavailable())?;
		let wrapped = self
			.master
			.seal(&[DATA_KEY_PURPOSE, &uuid_bytes(organization.uuid())], &key)
			.map_err(|_| unavailable())?;
		NewDataKey {
			organization_id: organization,
			wrapped,
		}
		.insert(tx)
		.await
		.map_err(|error| {
			if error.is_conflict() {
				rpc::Error::conflict(
					"vault_busy: this organisation's first secret is being stored right now; try \
					 again",
				)
			} else {
				error.into()
			}
		})?;

		Ok(Unwrapped {
			organization,
			key: Keys::single(DATA_KEY_ID, key),
		})
	}

	/// The organisation's data key, when it has one.
	///
	/// # Errors
	///
	/// `Server` (`vault_unavailable`) when the stored key doesn't open
	/// under any master key.
	pub(crate) async fn data_key(
		&self,
		db: &impl leptoskit_db::Read,
		organization: Id<Organization>,
	) -> rpc::Result<Option<Unwrapped>> {
		let Some(row) = DataKey::query()
			.filter(|key| key.organization_id.eq(organization))
			.first(db)
			.await?
		else {
			return Ok(None);
		};
		let opened = self
			.master
			.open(
				&[DATA_KEY_PURPOSE, &uuid_bytes(organization.uuid())],
				&row.wrapped,
			)
			.map_err(|_| unavailable())?;
		let key = <[u8; 32]>::try_from(opened).map_err(|_| unavailable())?;

		Ok(Some(Unwrapped {
			organization,
			key: Keys::single(DATA_KEY_ID, key),
		}))
	}
}

impl Unwrapped {
	/// What a value at `place` is bound to, each part length-prefixed by
	/// [`Keys::seal`].
	fn context(place: &Place<'_>) -> [Vec<u8>; 7] {
		[
			VALUE_PURPOSE.to_vec(),
			uuid_bytes(place.organization.uuid()).to_vec(),
			uuid_bytes(place.project.uuid()).to_vec(),
			uuid_bytes(place.environment.uuid()).to_vec(),
			uuid_bytes(place.secret.uuid()).to_vec(),
			place.name.as_bytes().to_vec(),
			place.version.to_be_bytes().to_vec(),
		]
	}

	/// `value`, sealed to `place`.
	///
	/// # Errors
	///
	/// `Server` (`vault_unavailable`) for a place in another organisation,
	/// or when there is no randomness.
	pub(crate) fn seal(&self, place: &Place<'_>, value: &str) -> rpc::Result<Vec<u8>> {
		if place.organization != self.organization {
			return Err(unavailable());
		}

		let context = Self::context(place);
		let parts: Vec<&[u8]> = context.iter().map(Vec::as_slice).collect();
		self.key
			.seal(&parts, value.as_bytes())
			.map_err(|_| unavailable())
	}

	/// The value `sealed` holds, when it was sealed to `place`.
	///
	/// # Errors
	///
	/// `Server` (`vault_unavailable`) when it wasn't, or was changed.
	pub(crate) fn open(&self, place: &Place<'_>, sealed: &[u8]) -> rpc::Result<String> {
		if place.organization != self.organization {
			return Err(unavailable());
		}

		let context = Self::context(place);
		let parts: Vec<&[u8]> = context.iter().map(Vec::as_slice).collect();
		let opened = self.key.open(&parts, sealed).map_err(|_| unavailable())?;
		String::from_utf8(opened).map_err(|_| unavailable())
	}
}

impl Unwrapped {
	/// A password's own context: its purpose differs from a secret's, so
	/// neither opens as the other.
	fn password_context(place: &DatabasePlace) -> [Vec<u8>; 5] {
		[
			PASSWORD_PURPOSE.to_vec(),
			uuid_bytes(place.organization.uuid()).to_vec(),
			uuid_bytes(place.environment.uuid()).to_vec(),
			uuid_bytes(place.database.uuid()).to_vec(),
			place.version.to_be_bytes().to_vec(),
		]
	}

	/// A database role's `password`, sealed to `place`.
	///
	/// # Errors
	///
	/// `Server` (`vault_unavailable`) for a place in another organisation,
	/// or when there is no randomness.
	pub(crate) fn seal_password(
		&self,
		place: &DatabasePlace,
		password: &str,
	) -> rpc::Result<Vec<u8>> {
		if place.organization != self.organization {
			return Err(unavailable());
		}

		let context = Self::password_context(place);
		let parts: Vec<&[u8]> = context.iter().map(Vec::as_slice).collect();
		self.key
			.seal(&parts, password.as_bytes())
			.map_err(|_| unavailable())
	}

	/// The password `sealed` holds, when it was sealed to `place`.
	///
	/// # Errors
	///
	/// `Server` (`vault_unavailable`) when it wasn't, or was changed.
	pub(crate) fn open_password(
		&self,
		place: &DatabasePlace,
		sealed: &[u8],
	) -> rpc::Result<String> {
		if place.organization != self.organization {
			return Err(unavailable());
		}

		let context = Self::password_context(place);
		let parts: Vec<&[u8]> = context.iter().map(Vec::as_slice).collect();
		let opened = self.key.open(&parts, sealed).map_err(|_| unavailable())?;
		String::from_utf8(opened).map_err(|_| unavailable())
	}
}

impl Unwrapped {
	fn tenant_context(place: &TenantPlace, value: TenantValue) -> [Vec<u8>; 6] {
		[
			TENANT_PURPOSE.to_vec(),
			value.label().to_vec(),
			uuid_bytes(place.organization.uuid()).to_vec(),
			uuid_bytes(place.environment.uuid()).to_vec(),
			uuid_bytes(place.tenant.uuid()).to_vec(),
			place.version.to_be_bytes().to_vec(),
		]
	}

	/// One of a tenant's values, sealed to `place`.
	///
	/// # Errors
	///
	/// `Server` (`vault_unavailable`) for a place in another organisation,
	/// or when there is no randomness.
	pub(crate) fn seal_tenant(
		&self,
		place: &TenantPlace,
		which: TenantValue,
		value: &str,
	) -> rpc::Result<Vec<u8>> {
		if place.organization != self.organization {
			return Err(unavailable());
		}

		let context = Self::tenant_context(place, which);
		let parts: Vec<&[u8]> = context.iter().map(Vec::as_slice).collect();
		self.key
			.seal(&parts, value.as_bytes())
			.map_err(|_| unavailable())
	}

	/// The tenant's value `sealed` holds, when it was sealed to `place` as
	/// `which`.
	///
	/// # Errors
	///
	/// `Server` (`vault_unavailable`) when it wasn't, or was changed.
	pub(crate) fn open_tenant(
		&self,
		place: &TenantPlace,
		which: TenantValue,
		sealed: &[u8],
	) -> rpc::Result<String> {
		if place.organization != self.organization {
			return Err(unavailable());
		}

		let context = Self::tenant_context(place, which);
		let parts: Vec<&[u8]> = context.iter().map(Vec::as_slice).collect();
		let opened = self.key.open(&parts, sealed).map_err(|_| unavailable())?;
		String::from_utf8(opened).map_err(|_| unavailable())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_tenants_values_open_only_as_themselves() {
		let organization = Id::from_uuid(Uuid::from_u128(1));
		let key = unwrapped(organization, 7);
		let home = TenantPlace {
			organization,
			environment: Id::from_uuid(Uuid::from_u128(3)),
			tenant: Id::from_uuid(Uuid::from_u128(4)),
			version: 1,
		};
		let sealed = key
			.seal_tenant(&home, TenantValue::Token, "c0ffee")
			.expect("sealed");
		assert_eq!(
			key.open_tenant(&home, TenantValue::Token, &sealed)
				.expect("opens"),
			"c0ffee"
		);
		assert!(
			key.open_tenant(&home, TenantValue::Password, &sealed)
				.is_err()
		);

		let elsewhere = [
			TenantPlace {
				environment: Id::from_uuid(Uuid::from_u128(9)),
				..home
			},
			TenantPlace {
				tenant: Id::from_uuid(Uuid::from_u128(9)),
				..home
			},
			TenantPlace { version: 2, ..home },
			TenantPlace {
				organization: Id::from_uuid(Uuid::from_u128(8)),
				..home
			},
		];

		for other in elsewhere {
			assert!(
				key.open_tenant(&other, TenantValue::Token, &sealed)
					.is_err(),
				"{other:?}"
			);
		}

		// Not a database's password at the same IDs.
		let as_database = DatabasePlace {
			organization,
			environment: home.environment,
			database: Id::from_uuid(home.tenant.uuid()),
			version: 1,
		};
		assert!(key.open_password(&as_database, &sealed).is_err());
	}

	fn unwrapped(organization: Id<Organization>, byte: u8) -> Unwrapped {
		Unwrapped {
			organization,
			key: Keys::single(DATA_KEY_ID, [byte; 32]),
		}
	}

	fn place(organization: Id<Organization>) -> Place<'static> {
		Place {
			organization,
			project: Id::from_uuid(Uuid::from_u128(2)),
			environment: Id::from_uuid(Uuid::from_u128(3)),
			secret: Id::from_uuid(Uuid::from_u128(4)),
			name: "API_KEY",
			version: 1,
		}
	}

	#[test]
	fn a_value_opens_only_where_it_was_sealed() {
		let organization = Id::from_uuid(Uuid::from_u128(1));
		let key = unwrapped(organization, 7);
		let home = place(organization);
		let sealed = key.seal(&home, "hunter2").expect("sealed");

		assert_eq!(key.open(&home, &sealed).expect("opens"), "hunter2");
		assert!(!sealed.windows(7).any(|window| window == b"hunter2"));

		let elsewhere = [
			Place {
				project: Id::from_uuid(Uuid::from_u128(9)),
				..home
			},
			Place {
				environment: Id::from_uuid(Uuid::from_u128(9)),
				..home
			},
			Place {
				secret: Id::from_uuid(Uuid::from_u128(9)),
				..home
			},
			Place {
				name: "API_KEY_2",
				..home
			},
			Place { version: 2, ..home },
		];

		for other in elsewhere {
			let error = key.open(&other, &sealed).expect_err("another place");
			assert!(error.to_string().contains("vault_unavailable"), "{error}");
			assert!(!error.to_string().contains("hunter2"));
		}

		// Another organisation's key, and this key asked about another
		// organisation.
		let stranger = Id::from_uuid(Uuid::from_u128(8));
		assert!(unwrapped(organization, 8).open(&home, &sealed).is_err());
		assert!(
			key.open(
				&Place {
					organization: stranger,
					..home
				},
				&sealed
			)
			.is_err()
		);
		assert!(
			key.seal(
				&Place {
					organization: stranger,
					..home
				},
				"x"
			)
			.is_err()
		);

		// A changed byte.
		let mut changed = sealed.clone();
		*changed.last_mut().expect("bytes") ^= 1;
		assert!(key.open(&home, &changed).is_err());
	}

	#[test]
	fn a_password_opens_only_for_its_database() {
		let organization = Id::from_uuid(Uuid::from_u128(1));
		let key = unwrapped(organization, 7);
		let home = DatabasePlace {
			organization,
			environment: Id::from_uuid(Uuid::from_u128(3)),
			database: Id::from_uuid(Uuid::from_u128(4)),
			version: 1,
		};
		let sealed = key.seal_password(&home, "c0ffee").expect("sealed");
		assert_eq!(key.open_password(&home, &sealed).expect("opens"), "c0ffee");

		let elsewhere = [
			DatabasePlace {
				environment: Id::from_uuid(Uuid::from_u128(9)),
				..home
			},
			DatabasePlace {
				database: Id::from_uuid(Uuid::from_u128(9)),
				..home
			},
			DatabasePlace { version: 2, ..home },
			DatabasePlace {
				organization: Id::from_uuid(Uuid::from_u128(8)),
				..home
			},
		];

		for other in elsewhere {
			assert!(key.open_password(&other, &sealed).is_err(), "{other:?}");
		}

		// A password is not a secret's value, even at the same IDs.
		let as_secret = Place {
			organization,
			project: Id::from_uuid(Uuid::from_u128(3)),
			environment: home.environment,
			secret: Id::from_uuid(home.database.uuid()),
			name: "DATABASE_URL",
			version: 1,
		};
		assert!(key.open(&as_secret, &sealed).is_err());
		assert!(
			unwrapped(organization, 8)
				.open_password(&home, &sealed)
				.is_err()
		);
	}

	#[test]
	fn two_seals_of_one_value_differ() {
		let organization = Id::from_uuid(Uuid::from_u128(1));
		let key = unwrapped(organization, 7);
		let home = place(organization);
		assert_ne!(
			key.seal(&home, "same").expect("sealed"),
			key.seal(&home, "same").expect("sealed")
		);
	}

	#[test]
	fn nothing_prints_a_key() {
		let organization = Id::from_uuid(Uuid::from_u128(1));
		assert_eq!(
			format!("{:?}", unwrapped(organization, 7)),
			"Unwrapped([redacted])"
		);
		let vault = Vault::new(Keys::single("2026-11", [9; 32]));
		assert_eq!(format!("{vault:?}"), "Vault { master: [\"2026-11\"] }");
	}
}
