//! Validating a first-party session that outlives its opening request.

use leptoskit_db::Tracked;
use leptoskit_rpc::Error;
use leptoskit_rpc::server::Identity;
use leptoskit_rpc::server::IdentityLease;

use crate::Account;
use crate::Auth;
use crate::Session;
use crate::SignedIn;

/// Only the hash is kept on a connection, never the presented token.
#[derive(Clone)]
pub(crate) struct Credential(pub Vec<u8>);

pub(crate) async fn lease(auth: &Auth, identity: &Identity) -> Result<IdentityLease, Error> {
	let Some(credential) = identity.context.get::<Credential>() else {
		// Delegated identities are validated by their own Identify adapter;
		// they are not first-party auth.sessions rows.
		return Ok(IdentityLease::default());
	};
	let me = identity
		.context
		.get::<SignedIn>()
		.ok_or_else(|| Error::unauthorized("the session has ended"))?;
	let db = Tracked::new(auth.inner.db.on_primary());
	let session = Session::query()
		.filter(|row| row.id.eq(me.session))
		.first(&db)
		.await
		.map_err(|_| Error::server("the session could not be checked"))?
		.ok_or_else(|| Error::unauthorized("the session has ended"))?;
	let account = Account::query()
		.filter(|row| row.id.eq(me.account))
		.first(&db)
		.await
		.map_err(|_| Error::server("the account could not be checked"))?;
	let now = leptoskit_db::now();

	if session.revoked_at.is_some()
		|| session.token_hash != credential.0
		|| session.account_id != me.account
		|| session.aal != me.aal
		|| session.mfa_at != me.mfa_at
		|| session.authenticated_at != me.authenticated_at
		|| session.expires_at <= now
		|| session.idle_expires_at <= now
		|| session
			.access_expires_at
			.is_some_and(|expires| expires <= now)
		|| account.is_none_or(|account| account.disabled_at.is_some())
	{
		return Err(Error::unauthorized("the session has ended"));
	}
	let expires = session
		.expires_at
		.min(session.idle_expires_at)
		.min(session.access_expires_at.unwrap_or(session.expires_at));
	let expires_at = std::time::SystemTime::from(expires);
	Ok(IdentityLease {
		continuity: Some(format!("{me:?}")),
		reads: db.into_reads(),
		expires_at: Some(expires_at),
		refresh_at: session
			.access_expires_at
			.filter(|access| *access < session.expires_at.min(session.idle_expires_at))
			.map(|access| {
				let issued = session.refreshed_at.unwrap_or(session.created_at);
				let lead = ((access - issued) / 2).min(chrono::TimeDelta::seconds(60));
				std::time::SystemTime::from(access - lead)
			}),
	})
}

#[cfg(test)]
mod tests {
	use leptoskit_db::Id;
	use leptoskit_rpc::server::Identity;

	use super::*;

	#[tokio::test]
	async fn a_lease_needs_a_session_that_still_matches() {
		let db = leptoskit_db::testing::sqlite().await;
		let auth = Auth::builder(&db, crate::Keys::single("test", [7; 32]))
			.mailer(crate::Mailbox::new())
			.build()
			.unwrap();

		// A credential without a person names no session.
		let mut context = crate::context(None);
		context = context.with(Credential(vec![0; 32]));
		let orphan = Identity::new(context);
		let refused = lease(&auth, &orphan).await.unwrap_err();
		assert!(
			format!("{refused:?}").contains("session has ended"),
			"{refused:?}"
		);

		// A credential and a person, but no session row.
		let me = SignedIn {
			account: Id::new(),
			session: Id::new(),
			aal: crate::Aal::One,
			authenticated_at: leptoskit_db::now(),
			mfa_at: None,
		};
		let mut context = crate::context(Some(me));
		context = context.with(Credential(vec![0; 32]));
		let gone = Identity::new(context);
		let refused = lease(&auth, &gone).await.unwrap_err();
		assert!(
			format!("{refused:?}").contains("session has ended"),
			"{refused:?}"
		);
	}
}
