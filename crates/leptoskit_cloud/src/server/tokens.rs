//! The bodies of [`crate::tokens`]'s operations.
//!
//! A token is never stronger than whoever makes it, who is always a
//! person: they give only the scopes their role grants. A token makes no
//! tokens, so none outlives the revocation of another.

use std::future::Future;

use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::audit;
use super::audit::Actor;
use super::authorize;
use super::identity::client;
use super::secret;
use crate::access::Permission;
use crate::access::TokenScope;
use crate::tables::ApiToken;
use crate::tables::NewApiToken;
use crate::tables::Project;
use crate::tokens::CreatedToken;
use crate::tokens::DEFAULT_LIFETIME_DAYS;
use crate::tokens::LIFETIMES_DAYS;
use crate::tokens::TOKEN_PREFIX;
use crate::tokens::TokenSummary;

/// The longest token name.
const MAX_NAME_LEN: usize = 64;

/// The most live tokens one organisation may have.
const MAX_LIVE_TOKENS: u64 = 200;

pub(crate) async fn tokens(
	cx: &rpc::QueryCx,
	organization: &str,
) -> rpc::Result<Vec<TokenSummary>> {
	let access = authorize::organization(
		cx.db(),
		cx.context(),
		organization,
		Permission::ManageTokens,
	)
	.await?;
	let organization_id = access.organization.id;
	let now = leptoskit_db::now();
	let rows = ApiToken::query()
		.filter(|token| {
			token.organization_id.eq(organization_id)
				& token.revoked_at.is_null()
				& token.expires_at.gt(now)
		})
		.order_by(|token| token.created_at.asc())
		.all(cx.db())
		.await?;
	let mut out = Vec::with_capacity(rows.len());

	for row in &rows {
		out.push(summary(cx.db(), row).await?);
	}

	Ok(out)
}

/// A token as managers see it, with its project's slug.
async fn summary(db: &impl leptoskit_db::Read, row: &ApiToken) -> rpc::Result<TokenSummary> {
	let project = match row.project_id {
		Some(project) => {
			Some(
				Project::query()
					.filter(|row| row.id.eq(project))
					.one(db)
					.await?
					.slug,
			)
		}
		None => None,
	};

	Ok(TokenSummary {
		id: row.id.uuid(),
		name: row.name.clone(),
		prefix: row.prefix.clone(),
		scopes: row.scopes.0.clone(),
		project,
		expires_at: row.expires_at,
		last_used_at: row.last_used_at,
		created_by: row.created_by.uuid(),
		created_at: row.created_at,
	})
}

/// The `Forbidden` for asking a token to be stronger than its maker.
fn too_strong(why: &str) -> rpc::Error {
	rpc::Error::forbidden(format!("token_too_strong: {why}"))
}

/// `create_token`'s body. It is an action so that nothing keeps its answer:
/// a mutation's answer is stored as its receipt when it comes with an
/// idempotency key, and this one holds the token's value. The writes still
/// happen in one serializable transaction, with the mutation runner's
/// retries, through [`MintToken`], which is never registered and so never
/// runs with a request ID.
pub(crate) async fn create_token(
	cx: &rpc::ActionCx,
	organization: String,
	name: String,
	scopes: Vec<TokenScope>,
	project: Option<String>,
	expires_in_days: Option<u32>,
) -> rpc::Result<CreatedToken> {
	cx.mutate(MintToken {
		organization,
		name,
		scopes,
		project,
		expires_in_days,
	})
	.await
}

/// The authorisation, the token's row, and its audit entry, in one
/// transaction. Only [`create_token`] runs it: it isn't served, so no
/// client calls it, and it never has a receipt.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct MintToken {
	organization: String,
	name: String,
	scopes: Vec<TokenScope>,
	project: Option<String>,
	expires_in_days: Option<u32>,
}

impl rpc::Operation for MintToken {
	type Context = rpc::MutationCx;
	type Output = CreatedToken;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	/// What the change log records the commit as.
	const NAME: &'static str = "create_token";

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<CreatedToken>> + Send {
		mint(cx, self)
	}
}

async fn mint(cx: &rpc::MutationCx, args: MintToken) -> rpc::Result<CreatedToken> {
	let MintToken {
		organization,
		name,
		scopes,
		project,
		expires_in_days,
	} = args;
	// Managing tokens takes a person, so no token gets past this to make
	// another: a token it made would outlive its own revocation.
	let access = authorize::organization(
		cx.tx(),
		cx.context(),
		&organization,
		Permission::ManageTokens,
	)
	.await?;
	let name = name.trim().to_owned();

	if name.is_empty() || name.chars().count() > MAX_NAME_LEN || name.chars().any(char::is_control)
	{
		return Err(rpc::Error::invalid(format!(
			"invalid_name: a token's name has 1 to {MAX_NAME_LEN} characters and no control \
			 characters"
		)));
	}

	let mut scopes = scopes;
	scopes.sort_unstable();
	scopes.dedup();

	if scopes.is_empty() {
		return Err(rpc::Error::invalid(
			"invalid_scopes: a token needs at least one scope",
		));
	}

	let days = expires_in_days.unwrap_or(DEFAULT_LIFETIME_DAYS);

	if !LIFETIMES_DAYS.contains(&days) {
		return Err(rpc::Error::invalid(format!(
			"invalid_lifetime: a token lives {} days",
			LIFETIMES_DAYS.map(|days| days.to_string()).join(", ")
		)));
	}

	// What the person may give, by role.
	if let Some(scope) = scopes.iter().find(|scope| !access.role.grants(**scope)) {
		return Err(too_strong(&format!(
			"a {} of `{organization}` can't give the `{scope}` scope",
			access.role
		)));
	}

	let organization_id = access.organization.id;
	let project = match project {
		Some(slug) => {
			let row = Project::query()
				.filter(|row| row.organization_id.eq(organization_id) & row.slug.eq(slug.clone()))
				.first(cx.tx())
				.await?
				.ok_or_else(|| {
					rpc::Error::not_found(format!(
						"project_not_found: there is no project `{slug}` in `{organization}` that \
						 you can see"
					))
				})?;
			Some(row)
		}
		None => None,
	};
	let now = leptoskit_db::now();
	let expires_at = now + chrono::TimeDelta::days(i64::from(days));

	// A live token's name is unique in its organisation, so a request sent
	// again after its answer was lost can't quietly make a second token the
	// caller can't tell from the first: it is refused, naming the first.
	if let Some(existing) = ApiToken::query()
		.filter(|token| {
			token.organization_id.eq(organization_id)
				& token.name.eq(name.clone())
				& token.revoked_at.is_null()
				& token.expires_at.gt(now)
		})
		.first(cx.tx())
		.await?
	{
		return Err(rpc::Error::conflict(format!(
			"token_name_taken: `{organization}` already has a live token `{name}`, \
			 {TOKEN_PREFIX}{}_…, made at {}. If it is from a request whose answer you didn't get, \
			 nobody has its value: revoke it ({}) and ask again. Otherwise choose another name",
			existing.prefix,
			existing.created_at.format("%Y-%m-%d %H:%M:%S UTC"),
			existing.id,
		)));
	}

	let live = ApiToken::query()
		.filter(|token| {
			token.organization_id.eq(organization_id)
				& token.revoked_at.is_null()
				& token.expires_at.gt(now)
		})
		.count(cx.tx())
		.await?;

	if live >= MAX_LIVE_TOKENS {
		return Err(rpc::Error::conflict(format!(
			"too_many_tokens: `{organization}` has {MAX_LIVE_TOKENS} live tokens; revoke some \
			 first"
		)));
	}

	let generated = secret::NewToken::generate()
		.map_err(|_| rpc::Error::server("the server has no randomness to make a token with"))?;
	let row = NewApiToken {
		organization_id,
		project_id: project.as_ref().map(|project| project.id),
		name,
		scopes: leptoskit_db::Json(scopes),
		prefix: generated.lookup.clone(),
		secret_hash: generated.secret_hash.clone(),
		expires_at,
		last_used_at: None,
		revoked_at: None,
		created_by: access.caller.account(),
	}
	.insert(cx.tx())
	.await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"token.create",
		Some(organization_id),
		row.project_id,
		Some(format!(
			"{} {} ({TOKEN_PREFIX}{})",
			row.id, row.name, row.prefix
		)),
		&client(cx.context()),
	)
	.await?;
	let token = summary(cx.tx(), &row).await?;

	Ok(CreatedToken {
		token,
		value: generated.value.clone(),
	})
}

pub(crate) async fn revoke_token(
	cx: &rpc::MutationCx,
	organization: &str,
	token: Uuid,
) -> rpc::Result<()> {
	let access = authorize::organization(
		cx.tx(),
		cx.context(),
		organization,
		Permission::ManageTokens,
	)
	.await?;
	let organization_id = access.organization.id;
	let id: Id<ApiToken> = Id::from_uuid(token);
	let row = ApiToken::query()
		.filter(|row| {
			row.id.eq(id) & row.organization_id.eq(organization_id) & row.revoked_at.is_null()
		})
		.first(cx.tx())
		.await?
		.ok_or_else(|| {
			rpc::Error::not_found(format!(
				"token_not_found: `{organization}` has no live token {token}"
			))
		})?;

	ApiToken::update()
		.filter(|row| row.id.eq(id))
		.set(|row| row.revoked_at, Some(leptoskit_db::now()))
		.run(cx.tx())
		.await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"token.revoke",
		Some(organization_id),
		row.project_id,
		Some(format!(
			"{} {} ({TOKEN_PREFIX}{})",
			row.id, row.name, row.prefix
		)),
		&client(cx.context()),
	)
	.await?;
	Ok(())
}
