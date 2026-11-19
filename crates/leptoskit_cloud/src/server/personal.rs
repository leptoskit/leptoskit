//! Each person's personal organisation, made the first time they call.
//!
//! Its slug comes from the local part of their primary verified address
//! (`ana@example.com` makes `ana`), or `personal` without one, because the
//! console shows it and a recognisable one is friendlier. When that is taken
//! or reserved, `ana-2` and `ana-3`, and after those `ana-` and ten random
//! hex characters (40 bits). Anyone who knows an address can take its
//! friendly slugs first, and anyone can take `personal`'s; that only moves
//! the person to a random suffix, which nobody can take in advance, so no
//! one can stop another person's first sign-in.
//!
//! The organisation's unique `personal_owner` column makes it once per
//! person, however many first requests race: the losers find the winner's.

use leptoskit_auth::Account;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;

use super::Services;
use super::audit;
use super::identity::ClientDescription;
use crate::access::Role;
use crate::tables::NewMembership;
use crate::tables::NewOrganization;
use crate::tables::Organization;

/// The plan a new organisation starts on.
pub(crate) const STARTING_PLAN: &str = "hobby";

/// How many friendly slugs to try: `ana`, `ana-2`, and `ana-3`.
const FRIENDLY_SLUGS: u32 = 3;

/// How many random slugs to try after those. The first is free unless 40
/// random bits match a slug someone holds, so running out of these means
/// the randomness is broken, not that the namespace is.
const RANDOM_SLUGS: u32 = 8;

/// How many random bytes a random slug's suffix has: ten hex characters.
const RANDOM_SUFFIX_BYTES: usize = 5;

/// Makes `account`'s personal organisation unless it exists.
///
/// # Errors
///
/// When the database fails, or the operating system has no randomness.
pub(crate) async fn ensure(
	services: &Services,
	account: Id<Account>,
	client: &ClientDescription,
) -> rpc::Result<()> {
	let db = services.db();

	if exists(db, account).await? {
		return Ok(());
	}

	let email = super::primary_email(db, account).await?;
	let base = base_slug(email.as_deref());
	let name = email
		.as_deref()
		.and_then(|email| email.split('@').next())
		.filter(|local| !local.is_empty())
		.unwrap_or("Personal")
		.to_owned();

	for attempt in 1..=FRIENDLY_SLUGS + RANDOM_SLUGS {
		let slug = if attempt == 1 {
			base.clone()
		} else if attempt <= FRIENDLY_SLUGS {
			suffixed(&base, &attempt.to_string())
		} else {
			let random = super::secret::random_hex(RANDOM_SUFFIX_BYTES)
				.map_err(|_| rpc::Error::server("the server has no randomness to name with"))?;
			suffixed(&base, &random)
		};

		if super::account::check_organization_slug(&slug).is_err() {
			continue;
		}

		match create(services, account, &slug, &name, client).await {
			Ok(()) => return Ok(()),
			// Another first request made it, or the slug is someone
			// else's: look again before trying the next one.
			Err(leptoskit_db::DbError::Conflict { .. }) => {
				if exists(db, account).await? {
					return Ok(());
				}
			}
			Err(error) => return Err(error.into()),
		}
	}

	Err(rpc::Error::server(format!(
		"{RANDOM_SLUGS} random slugs for a personal organisation were all taken, so the server's \
		 randomness is broken"
	)))
}

async fn exists(db: &impl leptoskit_db::Read, account: Id<Account>) -> rpc::Result<bool> {
	Ok(Organization::query()
		.filter(|organization| organization.personal_owner.eq(Some(account)))
		.first(db)
		.await?
		.is_some())
}

/// The organisation, its owner, and the audit entry, in one transaction.
async fn create(
	services: &Services,
	account: Id<Account>,
	slug: &str,
	name: &str,
	client: &ClientDescription,
) -> leptoskit_db::Result<()> {
	let tx = services.db().begin().await?;
	let result = async {
		let organization = NewOrganization {
			slug: slug.to_owned(),
			name: name.to_owned(),
			plan: STARTING_PLAN.to_owned(),
			personal_owner: Some(account),
			created_by: account,
		}
		.insert(&tx)
		.await?;
		NewMembership {
			organization_id: organization.id,
			account_id: account,
			role: Role::Owner,
		}
		.insert(&tx)
		.await?;
		audit::record(
			&tx,
			&audit::Actor::Person(account),
			"organization.create_personal",
			Some(organization.id),
			None,
			Some(slug.to_owned()),
			&client.0,
		)
		.await
	}
	.await;

	match result {
		Ok(()) => tx.commit().await,
		Err(error) => {
			// The transaction is dropped unfinished, which rolls it back.
			drop(tx);
			Err(error)
		}
	}
}

/// The slug to start from: the address's local part made a slug, or
/// `personal`.
fn base_slug(email: Option<&str>) -> String {
	email
		.and_then(|email| email.split('@').next())
		.and_then(|local| crate::slug::Slug::suggest(local).ok())
		.map(|slug| slug.as_str().to_owned())
		.filter(|slug| slug.len() >= crate::slug::SLUG_MIN_LEN)
		.unwrap_or_else(|| "personal".to_owned())
}

/// `base` with `-<suffix>`, cut so the whole stays a slug's length.
fn suffixed(base: &str, suffix: &str) -> String {
	let suffix = format!("-{suffix}");
	let keep = crate::slug::SLUG_MAX_LEN.saturating_sub(suffix.len());
	let mut stem: String = base.chars().take(keep).collect();
	while stem.ends_with('-') {
		stem.pop();
	}
	format!("{stem}{suffix}")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn slugs_come_from_the_address() {
		assert_eq!(base_slug(Some("ana@example.com")), "ana");
		assert_eq!(
			base_slug(Some("Ana.Lima+cloud@example.com")),
			"ana-lima-cloud"
		);
		assert_eq!(base_slug(Some("42@example.com")), "personal");
		assert_eq!(base_slug(Some("jo@example.com")), "personal");
		assert_eq!(base_slug(None), "personal");
		assert_eq!(suffixed("ana", "2"), "ana-2");
		assert_eq!(suffixed(&"a".repeat(40), "12").len(), 40);

		// A long stem is cut, and never left ending in a hyphen.
		let random = suffixed(&"ab-".repeat(20), "0123456789");
		assert_eq!(random, format!("{}-0123456789", "ab-".repeat(9) + "ab"));
		assert!(crate::slug::Slug::parse(&random).is_ok(), "{random}");
		assert!(random.len() <= crate::slug::SLUG_MAX_LEN);
	}
}
