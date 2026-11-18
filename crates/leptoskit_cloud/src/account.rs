//! Who is calling, the organisations they're in, and who else is in them:
//! `whoami`, organisations, members, and invitations.
//!
//! Organisations are named by slug in every argument, and the server finds
//! them again in the database for each call: a caller sees an organisation
//! only when they're in it, or, for a token, when it's the token's own. Any
//! other slug, including one that exists, answers "not found".

use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::access::Role;
use crate::access::TokenScope;

/// How the caller reached the cloud.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Via {
	/// A browser session, or a native app's session token.
	Web,
	/// The CLI, signed in with the device flow.
	Cli,
	/// An API token.
	Token,
}

/// An organisation, as one caller sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OrganizationSummary {
	pub id: Uuid,
	pub slug: String,
	pub name: String,
	/// The plan it is on, such as `hobby`.
	pub plan: String,
	/// Whether it is someone's personal organisation.
	pub personal: bool,
	/// The caller's role in it. A token has the role of the person it acts
	/// for, narrowed by its scopes.
	pub role: Role,
}

/// The API token a call came with: never its secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallerToken {
	pub id: Uuid,
	pub name: String,
	/// The non-secret part of its value, `lkc_<prefix>_…`.
	pub prefix: String,
	/// The organisation it belongs to.
	pub organization: String,
	/// The one project it may touch, when it is bound to one.
	pub project: Option<String>,
	pub scopes: Vec<TokenScope>,
	pub expires_at: Timestamp,
}

/// Who is calling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WhoAmI {
	/// The person: the one signed in, or the one a token acts for.
	pub account: Uuid,
	/// Their primary verified address, for a person signed in themselves.
	pub email: Option<String>,
	pub via: Via,
	/// Whether they run the cloud itself. Always `false` for a token.
	pub operator: bool,
	/// The organisations the caller can see, with their role in each.
	pub organizations: Vec<OrganizationSummary>,
	/// The token, when the call came with one.
	pub token: Option<CallerToken>,
}

/// Someone in an organisation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Member {
	pub account: Uuid,
	/// Their primary verified address, when they have one.
	pub email: Option<String>,
	pub role: Role,
	pub joined_at: Timestamp,
}

/// An invitation that hasn't been accepted, revoked, or let expire. Never
/// its link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PendingInvitation {
	pub id: Uuid,
	pub email: String,
	pub role: Role,
	pub invited_by: Uuid,
	pub expires_at: Timestamp,
	pub created_at: Timestamp,
}

/// Who is calling: the person, how they reached the cloud, the
/// organisations they can see with their role in each, and a token's
/// scopes.
///
/// # Errors
///
/// `Unauthorized` for a caller who isn't signed in.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "account")]
#[rpc::query]
pub async fn whoami(cx: &rpc::QueryCx) -> rpc::Result<WhoAmI> {
	crate::server::account::whoami(cx).await
}

/// The organisations the caller can see: every one a person is in, or a
/// token's own.
///
/// # Errors
///
/// `Unauthorized` for a caller who isn't signed in.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "account")]
#[rpc::query]
pub async fn organizations(cx: &rpc::QueryCx) -> rpc::Result<Vec<OrganizationSummary>> {
	crate::server::account::organizations(cx).await
}

/// Makes an organisation, with the caller as its owner. Only a person may:
/// a token belongs to an organisation already.
///
/// # Arguments
///
/// - `slug`: what `/<org>` will call it: 3 to 40 lower-case letters, digits,
///   and hyphens, starting with a letter.
/// - `name`: what the console shows.
///
/// # Errors
///
/// `Invalid` for a slug or name that breaks the rules or is reserved,
/// `Conflict` (`slug_taken`) when the slug is in use, `Forbidden` for a token.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "account")]
#[rpc::mutation]
pub async fn create_organization(
	cx: &rpc::MutationCx,
	slug: String,
	name: String,
) -> rpc::Result<OrganizationSummary> {
	crate::server::account::create_organization(cx, slug, name).await
}

/// Everyone in an organisation, with their role.
///
/// # Errors
///
/// `NotFound` for an organisation the caller can't see.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "account")]
#[rpc::query]
pub async fn members(cx: &rpc::QueryCx, organization: String) -> rpc::Result<Vec<Member>> {
	crate::server::account::members(cx, &organization).await
}

/// Emails `email` a link that adds them to the organisation with `role`,
/// once they sign in with that address. The link works once and lasts
/// seven days. Inviting the same address again replaces the earlier link.
///
/// # Errors
///
/// `Forbidden` unless the caller manages members (and, to invite an owner,
/// is one); `Conflict` (`already_member`) when the address belongs to a
/// member; `Invalid` for an address that isn't one.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "account")]
#[rpc::mutation]
pub async fn invite_member(
	cx: &rpc::MutationCx,
	organization: String,
	email: String,
	role: Role,
) -> rpc::Result<PendingInvitation> {
	crate::server::account::invite_member(cx, &organization, email, role).await
}

/// Accepts an invitation with the token from its link, as the signed-in
/// person, who must have verified the invited address. Accepting twice
/// answers the same.
///
/// # Errors
///
/// `NotFound` (`invitation_invalid`) for a token that is unknown, revoked,
/// expired, or used by someone else; `Forbidden` (`wrong_email`) when the
/// caller hasn't verified the invited address.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "account")]
#[rpc::mutation]
pub async fn accept_invitation(
	cx: &rpc::MutationCx,
	token: String,
) -> rpc::Result<OrganizationSummary> {
	crate::server::account::accept_invitation(cx, token).await
}

/// The invitations waiting for an answer.
///
/// # Errors
///
/// `Forbidden` unless the caller manages members.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "account")]
#[rpc::query]
pub async fn invitations(
	cx: &rpc::QueryCx,
	organization: String,
) -> rpc::Result<Vec<PendingInvitation>> {
	crate::server::account::invitations(cx, &organization).await
}

/// Revokes a pending invitation: its link stops working at once.
///
/// # Errors
///
/// `NotFound` for an invitation that isn't this organisation's, or isn't
/// pending.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "account")]
#[rpc::mutation]
pub async fn revoke_invitation(
	cx: &rpc::MutationCx,
	organization: String,
	invitation: Uuid,
) -> rpc::Result<()> {
	crate::server::account::revoke_invitation(cx, &organization, invitation).await
}

/// Changes a member's role. Changing it to the role they have changes
/// nothing.
///
/// # Errors
///
/// `Forbidden` unless the caller manages members, and owns the organisation
/// when the change involves the owner role; `Conflict` (`last_owner`) when
/// it would leave the organisation without an owner.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "account")]
#[rpc::mutation]
pub async fn set_member_role(
	cx: &rpc::MutationCx,
	organization: String,
	account: Uuid,
	role: Role,
) -> rpc::Result<()> {
	crate::server::account::set_member_role(cx, &organization, account, role).await
}

/// Removes someone from the organisation, and revokes the tokens they
/// made in it. Anyone may remove themselves.
///
/// # Errors
///
/// As [`set_member_role`].
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "account")]
#[rpc::mutation]
pub async fn remove_member(
	cx: &rpc::MutationCx,
	organization: String,
	account: Uuid,
) -> rpc::Result<()> {
	crate::server::account::remove_member(cx, &organization, account).await
}
