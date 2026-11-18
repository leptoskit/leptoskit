//! Who is calling: a session, the CLI's device-flow token, or an API token,
//! in that order. See the [module docs](super) for the rules.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use std::time::SystemTime;

use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::CurrentSession;
use leptoskit_auth::SignedIn;
use leptoskit_db::Context;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::Identify;
use leptoskit_rpc::server::Identity;
use leptoskit_rpc::server::IdentityLease;

use super::Services;
use super::secret;
use crate::access::TokenScope;
use crate::account::Via;
use crate::tables::ApiToken;
use crate::tables::Membership;
use crate::tables::Organization;
use crate::tables::Project;
use crate::tokens::TokenValue;

/// How often a token's `last_used_at` is written: at most once in this
/// long, so a busy token doesn't write on every request.
const LAST_USED_PRECISION: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

/// How long a live connection keeps an identity that has no session behind
/// it before checking it again: a token's revocation reaches a socket at
/// most this late.
const LEASE: Duration = Duration::from_mins(1);

/// The longest description of a client the audit log keeps.
const MAX_CLIENT_LEN: usize = 300;

/// Who is calling, as every operation's body reads it from the context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Caller {
	/// A person: signed in themselves, or through the CLI.
	Person {
		account: Id<Account>,
		/// [`Via::Web`] or [`Via::Cli`].
		via: Via,
	},
	/// An API token, acting for the person who made it.
	Token(TokenCaller),
}

impl Caller {
	/// The person: the one signed in, or the one a token acts for.
	#[must_use]
	pub fn account(&self) -> Id<Account> {
		match self {
			Self::Person { account, .. } => *account,
			Self::Token(token) => token.created_by,
		}
	}

	/// How the caller reached the cloud.
	#[must_use]
	pub fn via(&self) -> Via {
		match self {
			Self::Person { via, .. } => *via,
			Self::Token(_) => Via::Token,
		}
	}

	/// The token, for a token's call.
	#[must_use]
	pub fn token(&self) -> Option<&TokenCaller> {
		match self {
			Self::Person { .. } => None,
			Self::Token(token) => Some(token),
		}
	}
}

/// A live API token, as it was when the call began. Never its secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenCaller {
	pub id: Id<ApiToken>,
	pub name: String,
	pub prefix: String,
	pub organization: Id<Organization>,
	pub project: Option<Id<Project>>,
	pub scopes: Vec<TokenScope>,
	pub created_by: Id<Account>,
	pub expires_at: Timestamp,
}

/// Where a call came from, as the audit log records it: the web and the
/// browser's user agent, the CLI, or an API token's name and prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientDescription(pub String);

impl ClientDescription {
	fn new(text: String) -> Self {
		let mut text = text;
		if text.len() > MAX_CLIENT_LEN {
			let mut end = MAX_CLIENT_LEN;
			while !text.is_char_boundary(end) {
				end -= 1;
			}
			text.truncate(end);
		}
		Self(text)
	}
}

/// The control plane's [`Identify`]: sessions through `leptoskit_auth`,
/// the CLI's device-flow token, and API tokens. It also makes a person's
/// personal organisation the first time they call, and adds the
/// [`Services`] to every caller's context.
pub struct CloudIdentify {
	auth: Auth,
	services: Services,
	cli_client_id: String,
}

impl CloudIdentify {
	/// Identifies callers with `auth`'s sessions, the device-flow tokens of
	/// the client `cli_client_id`, and API tokens.
	pub fn new(auth: Auth, services: Services, cli_client_id: impl Into<String>) -> Self {
		Self {
			auth,
			services,
			cli_client_id: cli_client_id.into(),
		}
	}

	/// A person's identity: `context`, which already says who they are to
	/// policies, with the caller and services added, and their personal
	/// organisation made if this is their first call.
	async fn person(
		&self,
		context: Context,
		account: Id<Account>,
		via: Via,
		client: ClientDescription,
	) -> rpc::Result<Context> {
		super::personal::ensure(&self.services, account, &client).await?;
		Ok(context
			.with(Caller::Person { account, via })
			.with(client)
			.with(self.services.clone()))
	}

	/// The API token `value` names, when it is live: known, matching,
	/// neither revoked nor expired, and still acting for a member.
	async fn token(&self, value: &str) -> rpc::Result<Option<TokenCaller>> {
		let Some(value) = TokenValue::parse(value) else {
			return Ok(None);
		};
		let db = self.services.db();
		let Some(row) = ApiToken::query()
			.filter(|token| token.prefix.eq(value.lookup))
			.first(db)
			.await?
		else {
			return Ok(None);
		};

		if !secret::matches(value.secret, &row.secret_hash) {
			return Ok(None);
		}

		let now = leptoskit_db::now();

		if row.revoked_at.is_some() || row.expires_at <= now {
			return Ok(None);
		}

		let member = Membership::query()
			.filter(|member| {
				member.organization_id.eq(row.organization_id)
					& member.account_id.eq(row.created_by)
			})
			.first(db)
			.await?;

		if member.is_none() {
			return Ok(None);
		}

		if row
			.last_used_at
			.is_none_or(|used| now - used >= LAST_USED_PRECISION)
		{
			ApiToken::update()
				.filter(|token| token.id.eq(row.id))
				.set(|token| token.last_used_at, Some(now))
				.run(db)
				.await?;
		}

		Ok(Some(TokenCaller {
			id: row.id,
			name: row.name,
			prefix: row.prefix,
			organization: row.organization_id,
			project: row.project_id,
			scopes: row.scopes.0,
			created_by: row.created_by,
			expires_at: row.expires_at,
		}))
	}
}

impl Identify for CloudIdentify {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> Pin<Box<dyn Future<Output = rpc::Result<Identity>> + Send + 'a>> {
		Box::pin(async move {
			// 2. A third-party access token reached here only because
			// `leptoskit_idp`'s layer verified it into a delegation. Only the
			// CLI's counts; any other app's is nobody.
			if let Some(delegation) = request
				.extensions
				.get::<CurrentSession>()
				.and_then(|current| current.delegation.as_ref())
			{
				let Some(signed_in) = delegation
					.signed_in
					.clone()
					.filter(|_| delegation.client_id == self.cli_client_id)
				else {
					return Ok(Identity::anonymous());
				};
				let account = signed_in.account;
				let context = self
					.person(
						leptoskit_auth::context(Some(signed_in)),
						account,
						Via::Cli,
						ClientDescription::new("cli".to_owned()),
					)
					.await?;
				return Ok(Identity::new(context).with_caller(format!("cli:{account}")));
			}

			// 1. A session, as `leptoskit_auth` resolves it: the layer's, or
			// the request's own cookie or bearer.
			let session = Identify::identify(&self.auth, request).await?;

			if let Some(signed_in) = session.context.get::<SignedIn>().cloned() {
				let agent = request
					.headers
					.get(http::header::USER_AGENT)
					.and_then(|agent| agent.to_str().ok())
					.unwrap_or("unknown");
				let context = self
					.person(
						session.context,
						signed_in.account,
						Via::Web,
						ClientDescription::new(format!("web: {agent}")),
					)
					.await?;
				let identity = Identity::new(context);
				return Ok(match session.caller {
					Some(caller) => identity.with_caller(caller),
					None => identity,
				});
			}

			// 3. An API token.
			let bearer = request
				.headers
				.get(http::header::AUTHORIZATION)
				.and_then(|value| value.to_str().ok())
				.and_then(|value| value.strip_prefix("Bearer "));

			if let Some(token) = match bearer {
				Some(bearer) => self.token(bearer.trim()).await?,
				None => None,
			} {
				let client =
					ClientDescription::new(format!("token: {} (lkc_{})", token.name, token.prefix));
				let caller = format!("token:{}", token.id);
				let context = Context::new()
					.with(Caller::Token(token))
					.with(client)
					.with(self.services.clone());
				return Ok(Identity::new(context).with_caller(caller));
			}

			Ok(Identity::anonymous())
		})
	}

	fn lease<'a>(
		&'a self,
		identity: &'a Identity,
	) -> Pin<Box<dyn Future<Output = rpc::Result<IdentityLease>> + Send + 'a>> {
		Box::pin(async move {
			match identity.context.get::<Caller>() {
				// A session's lease is `leptoskit_auth`'s: it follows the
				// session row, so signing out reaches the socket at once.
				Some(Caller::Person { via: Via::Web, .. }) => {
					Identify::lease(&self.auth, identity).await
				}
				Some(Caller::Person { .. }) => {
					Ok(IdentityLease {
						expires_at: Some(SystemTime::now() + LEASE),
						..IdentityLease::default()
					})
				}
				Some(Caller::Token(token)) => {
					let still = ApiToken::query()
						.filter(|row| {
							row.id.eq(token.id)
								& row.revoked_at.is_null()
								& row.expires_at.gt(leptoskit_db::now())
						})
						.first(self.services.db())
						.await?;

					if still.is_none() {
						return Err(rpc::Error::unauthorized("the token was revoked or expired"));
					}

					let expires: SystemTime = token.expires_at.into();
					Ok(IdentityLease {
						expires_at: Some((SystemTime::now() + LEASE).min(expires)),
						..IdentityLease::default()
					})
				}
				None => Ok(IdentityLease::default()),
			}
		})
	}
}

/// The caller, or `Unauthorized` for an anonymous one.
///
/// # Errors
///
/// [`rpc::Error::Unauthorized`] when nobody is signed in.
pub(crate) fn caller(context: &Context) -> rpc::Result<&Caller> {
	context.get::<Caller>().ok_or_else(|| {
		rpc::Error::unauthorized(
			"sign_in_required: sign in, or call with an API token (`lk cloud login`)",
		)
	})
}

/// The caller's client, for the audit log.
pub(crate) fn client(context: &Context) -> String {
	context
		.get::<ClientDescription>()
		.map_or_else(|| "unknown".to_owned(), |client| client.0.clone())
}
