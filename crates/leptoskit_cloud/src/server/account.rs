//! The bodies of [`crate::account`]'s operations.

use leptoskit_auth::Account;
use leptoskit_auth::Mail;
use leptoskit_auth::Mailer as _;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use uuid::Uuid;

use super::audit;
use super::audit::Actor;
use super::authorize;
use super::identity::Caller;
use super::identity::caller;
use super::identity::client;
use super::secret;
use super::services;
use crate::access::Permission;
use crate::access::RESERVED_ORGANIZATION_SLUGS;
use crate::access::Role;
use crate::account::CallerToken;
use crate::account::Member;
use crate::account::OrganizationSummary;
use crate::account::PendingInvitation;
use crate::account::WhoAmI;
use crate::tables::ApiToken;
use crate::tables::Invitation;
use crate::tables::Membership;
use crate::tables::NewInvitation;
use crate::tables::NewMembership;
use crate::tables::NewOrganization;
use crate::tables::Organization;
use crate::tables::Project;

/// How long an invitation's link works.
const INVITATION_LIFETIME: chrono::TimeDelta = chrono::TimeDelta::days(7);

/// The longest organisation name.
const MAX_NAME_LEN: usize = 100;

/// The most pending invitations one organisation may have, so nobody can
/// grow the table, or send mail, without bound.
const MAX_PENDING_INVITATIONS: u64 = 100;

pub(crate) async fn whoami(cx: &rpc::QueryCx) -> rpc::Result<WhoAmI> {
	let caller = caller(cx.context())?;
	let services = services(cx.context())?;
	let account = caller.account();
	let organizations = visible_organizations(cx.db(), caller).await?;

	let (email, operator, token) = match caller {
		Caller::Person { .. } => {
			(
				super::primary_email(services.db(), account).await?,
				super::is_operator(cx.db(), account).await?,
				None,
			)
		}
		Caller::Token(token) => {
			let organization = Organization::query()
				.filter(|row| row.id.eq(token.organization))
				.one(cx.db())
				.await?;
			let project = match token.project {
				Some(project) => {
					Some(
						Project::query()
							.filter(|row| row.id.eq(project))
							.one(cx.db())
							.await?
							.slug,
					)
				}
				None => None,
			};
			(
				None,
				false,
				Some(CallerToken {
					id: token.id.uuid(),
					name: token.name.clone(),
					prefix: token.prefix.clone(),
					organization: organization.slug,
					project,
					scopes: token.scopes.clone(),
					expires_at: token.expires_at,
				}),
			)
		}
	};

	Ok(WhoAmI {
		account: account.uuid(),
		email,
		via: caller.via(),
		operator,
		organizations,
		token,
	})
}

pub(crate) async fn organizations(cx: &rpc::QueryCx) -> rpc::Result<Vec<OrganizationSummary>> {
	let caller = caller(cx.context())?;
	visible_organizations(cx.db(), caller).await
}

/// Every organisation `caller` can see, with their role in each.
async fn visible_organizations(
	db: &impl leptoskit_db::Read,
	caller: &Caller,
) -> rpc::Result<Vec<OrganizationSummary>> {
	let account = caller.account();
	let memberships = Membership::query()
		.filter(|member| member.account_id.eq(account))
		.order_by(|member| member.created_at.asc())
		.all(db)
		.await?;
	let mut out = Vec::with_capacity(memberships.len());

	for membership in memberships {
		if caller
			.token()
			.is_some_and(|token| token.organization != membership.organization_id)
		{
			continue;
		}

		let organization = Organization::query()
			.filter(|row| row.id.eq(membership.organization_id))
			.one(db)
			.await?;
		out.push(summary(&organization, membership.role));
	}

	Ok(out)
}

pub(crate) fn summary(organization: &Organization, role: Role) -> OrganizationSummary {
	OrganizationSummary {
		id: organization.id.uuid(),
		slug: organization.slug.clone(),
		name: organization.name.clone(),
		plan: organization.plan.clone(),
		personal: organization.personal_owner.is_some(),
		role,
	}
}

/// Checks a slug for an organisation: the slug rules, and none of the
/// console's own route words.
///
/// # Errors
///
/// `Invalid`, saying which rule it breaks.
pub(crate) fn check_organization_slug(slug: &str) -> rpc::Result<()> {
	crate::slug::Slug::parse(slug)
		.map_err(|error| rpc::Error::invalid(format!("invalid_slug: {error}")))?;

	if RESERVED_ORGANIZATION_SLUGS.contains(&slug) {
		return Err(rpc::Error::invalid(format!(
			"invalid_slug: `{slug}` is reserved for the console's own pages"
		)));
	}

	Ok(())
}

/// Checks a display name: 1 to 100 characters, not only spaces, and no
/// control characters.
pub(crate) fn check_name(name: &str) -> rpc::Result<String> {
	let name = name.trim();

	if name.is_empty() || name.chars().count() > MAX_NAME_LEN || name.chars().any(char::is_control)
	{
		return Err(rpc::Error::invalid(format!(
			"invalid_name: a name has 1 to {MAX_NAME_LEN} characters and no control characters"
		)));
	}

	Ok(name.to_owned())
}

pub(crate) async fn create_organization(
	cx: &rpc::MutationCx,
	slug: String,
	name: String,
) -> rpc::Result<OrganizationSummary> {
	let caller = caller(cx.context())?;

	let Caller::Person { account, .. } = caller else {
		return Err(authorize::person_required("create an organisation"));
	};

	check_organization_slug(&slug)?;
	let name = check_name(&name)?;
	let organization = NewOrganization {
		slug: slug.clone(),
		name,
		plan: super::personal::STARTING_PLAN.to_owned(),
		personal_owner: None,
		created_by: *account,
	}
	.insert(cx.tx())
	.await
	.map_err(|error| slug_taken(error, &slug))?;
	NewMembership {
		organization_id: organization.id,
		account_id: *account,
		role: Role::Owner,
	}
	.insert(cx.tx())
	.await?;
	audit::record(
		cx.tx(),
		&Actor::from(caller),
		"organization.create",
		Some(organization.id),
		None,
		Some(slug),
		&client(cx.context()),
	)
	.await?;

	Ok(summary(&organization, Role::Owner))
}

/// A unique violation on a slug as `Conflict` (`slug_taken`); anything else
/// as itself.
pub(crate) fn slug_taken(error: leptoskit_db::DbError, slug: &str) -> rpc::Error {
	match error {
		leptoskit_db::DbError::Conflict { .. } => {
			rpc::Error::conflict(format!("slug_taken: `{slug}` is taken; choose another"))
		}
		other => other.into(),
	}
}

pub(crate) async fn members(cx: &rpc::QueryCx, organization: &str) -> rpc::Result<Vec<Member>> {
	let access = authorize::organization(
		cx.db(),
		cx.context(),
		organization,
		Permission::ViewOrganization,
	)
	.await?;
	let services = services(cx.context())?;
	let organization_id = access.organization.id;
	let rows = Membership::query()
		.filter(|member| member.organization_id.eq(organization_id))
		.order_by(|member| member.created_at.asc())
		.all(cx.db())
		.await?;
	let mut out = Vec::with_capacity(rows.len());

	for row in rows {
		out.push(Member {
			account: row.account_id.uuid(),
			email: super::primary_email(services.db(), row.account_id).await?,
			role: row.role,
			joined_at: row.created_at,
		});
	}

	Ok(out)
}

/// An address as invitations keep it: trimmed, lowercased, and checked.
fn normalize_email(email: &str) -> rpc::Result<String> {
	let normalized = email.trim().to_ascii_lowercase();
	leptoskit_db::Email::parse(&normalized)
		.map_err(|_| rpc::Error::invalid("invalid_email: that isn't an email address"))?;
	Ok(normalized)
}

pub(crate) async fn invite_member(
	cx: &rpc::MutationCx,
	organization: &str,
	email: String,
	role: Role,
) -> rpc::Result<PendingInvitation> {
	let access = authorize::organization(
		cx.tx(),
		cx.context(),
		organization,
		Permission::ManageMembers,
	)
	.await?;
	access.require_to_give(role)?;
	let services = services(cx.context())?;
	let email = normalize_email(&email)?;
	let organization_id = access.organization.id;
	let now = leptoskit_db::now();
	// Inviting an address again replaces its pending invitations, which is
	// revoking each: the caller needs the authority to revoke every one,
	// so an admin can't cancel an owner's invitation this way.
	let replaced = Invitation::query()
		.filter(|row| {
			row.organization_id.eq(organization_id)
				& row.email.eq(email.clone())
				& row.accepted_at.is_null()
				& row.revoked_at.is_null()
				& row.expires_at.gt(now)
		})
		.all(cx.tx())
		.await?;

	for row in &replaced {
		access.require_to_give(row.role)?;
	}

	for row in &replaced {
		let id = row.id;
		Invitation::update()
			.filter(|row| row.id.eq(id))
			.set(|row| row.revoked_at, Some(now))
			.run(cx.tx())
			.await?;
	}

	let pending = Invitation::query()
		.filter(|row| {
			row.organization_id.eq(organization_id)
				& row.accepted_at.is_null()
				& row.revoked_at.is_null()
				& row.expires_at.gt(now)
		})
		.count(cx.tx())
		.await?;

	if pending >= MAX_PENDING_INVITATIONS {
		return Err(rpc::Error::conflict(format!(
			"too_many_invitations: `{organization}` has {MAX_PENDING_INVITATIONS} pending \
			 invitations; revoke some first"
		)));
	}

	let token = secret::random_hex(32)
		.map_err(|_| rpc::Error::server("the server has no randomness to make a link with"))?;
	let invitation = NewInvitation {
		organization_id,
		email: email.clone(),
		role,
		token_hash: secret::hash(&token),
		expires_at: now + INVITATION_LIFETIME,
		invited_by: access.caller.account(),
		accepted_at: None,
		accepted_by: None,
		revoked_at: None,
	}
	.insert(cx.tx())
	.await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"invitation.create",
		Some(organization_id),
		None,
		Some(format!("{} {email} as {role}", invitation.id)),
		&client(cx.context()),
	)
	.await?;

	// The address is in the entry above; an address can be long enough that
	// repeating it here wouldn't fit.
	for row in &replaced {
		audit::record(
			cx.tx(),
			&Actor::from(&access.caller),
			"invitation.revoke",
			Some(organization_id),
			None,
			Some(format!("{} (replaced by {})", row.id, invitation.id)),
			&client(cx.context()),
		)
		.await?;
	}

	// The mail names the inviter by their own address, which the transaction
	// may read (only a person invites). Nobody else's address is read here:
	// that would take a second connection while this transaction holds one.
	let inviter = super::primary_email(cx.tx(), access.caller.account()).await?;
	let mail = Mail {
		to: email,
		subject: format!("Join {} on LeptosKit Cloud", access.organization.name),
		text: invitation_text(
			inviter.as_deref(),
			&access.organization.name,
			role,
			&format!("{}/invitations/{token}", services.origin()),
			invitation.expires_at,
		),
	};
	let mailer = services.mailer().clone();
	// The mail goes once the invitation exists, and never for a retried
	// run. A failure to send is the mailer's to log; the invitation can be
	// sent again.
	cx.after_commit(async move {
		let _ = mailer.send(mail).await;
	});

	Ok(pending_of(&invitation))
}

/// The plain-text body of an invitation.
fn invitation_text(
	inviter: Option<&str>,
	organization: &str,
	role: Role,
	link: &str,
	expires_at: leptoskit_db::Timestamp,
) -> String {
	let who = inviter.unwrap_or("Someone");
	format!(
		"{who} invited you to join {organization} on LeptosKit Cloud as {article} {role}.\n\nTo \
		 accept, open this link and sign in with this address:\n\n{link}\n\nThe link works once, \
		 until {expires}. If you weren't expecting this, you can ignore it.\n",
		article = if role == Role::Owner || role == Role::Admin {
			"an"
		} else {
			"a"
		},
		expires = expires_at.format("%-d %B %Y, %H:%M UTC"),
	)
}

fn pending_of(invitation: &Invitation) -> PendingInvitation {
	PendingInvitation {
		id: invitation.id.uuid(),
		email: invitation.email.clone(),
		role: invitation.role,
		invited_by: invitation.invited_by.uuid(),
		expires_at: invitation.expires_at,
		created_at: invitation.created_at,
	}
}

pub(crate) async fn accept_invitation(
	cx: &rpc::MutationCx,
	token: String,
) -> rpc::Result<OrganizationSummary> {
	let caller = caller(cx.context())?;

	let Caller::Person { account, .. } = caller else {
		return Err(authorize::person_required("accept an invitation"));
	};

	let account = *account;
	// Unknown, revoked, expired, and someone else's all read the same.
	let invalid = || {
		rpc::Error::not_found(
			"invitation_invalid: this invitation link isn't valid; ask for a new one",
		)
	};
	let hash = secret::hash(token.trim());
	let invitation = Invitation::query()
		.filter(|row| row.token_hash.eq(hash))
		.first(cx.tx())
		.await?
		.ok_or_else(invalid)?;
	let now = leptoskit_db::now();
	let organization = Organization::query()
		.filter(|row| row.id.eq(invitation.organization_id))
		.one(cx.tx())
		.await?;

	if let Some(accepted_by) = invitation.accepted_by {
		if accepted_by != account {
			return Err(invalid());
		}

		// Accepting again answers the same.
		let membership = membership_of(cx.tx(), organization.id, account)
			.await?
			.ok_or_else(invalid)?;
		return Ok(summary(&organization, membership.role));
	}

	if invitation.revoked_at.is_some() || invitation.expires_at <= now {
		return Err(invalid());
	}

	// An invitation is only as good as its inviter's authority now: one
	// whose inviter has left, or holds a role that can no longer give the
	// role it offers, is no invitation. Demoting or removing someone
	// revokes these too; this holds however their role changed.
	let inviter = membership_of(cx.tx(), organization.id, invitation.invited_by).await?;

	if !inviter.is_some_and(|inviter| inviter.role.may_give(invitation.role)) {
		return Err(invalid());
	}

	// The caller must have verified the invited address: their own rows,
	// which the auth tables' policy lets them read.
	let Ok(address) = leptoskit_db::Email::parse(&invitation.email) else {
		return Err(invalid());
	};
	let verified = leptoskit_auth::EmailAddress::query()
		.filter(|row| {
			row.account_id.eq(account) & row.address.eq(address) & row.verified_at.is_not_null()
		})
		.first(cx.tx())
		.await?;

	if verified.is_none() {
		return Err(rpc::Error::forbidden(
			"wrong_email: sign in with the address this invitation was sent to, and verify it",
		));
	}

	// Someone already in keeps their role.
	let role = if let Some(existing) = membership_of(cx.tx(), organization.id, account).await? {
		existing.role
	} else {
		NewMembership {
			organization_id: organization.id,
			account_id: account,
			role: invitation.role,
		}
		.insert(cx.tx())
		.await?;
		invitation.role
	};

	Invitation::update()
		.filter(|row| row.id.eq(invitation.id))
		.set(|row| row.accepted_at, Some(now))
		.set(|row| row.accepted_by, Some(account))
		.run(cx.tx())
		.await?;
	audit::record(
		cx.tx(),
		&Actor::from(caller),
		"invitation.accept",
		Some(organization.id),
		None,
		Some(format!("{} as {role}", invitation.id)),
		&client(cx.context()),
	)
	.await?;

	Ok(summary(&organization, role))
}

async fn membership_of(
	db: &impl leptoskit_db::Read,
	organization: Id<Organization>,
	account: Id<Account>,
) -> rpc::Result<Option<Membership>> {
	Ok(Membership::query()
		.filter(|member| member.organization_id.eq(organization) & member.account_id.eq(account))
		.first(db)
		.await?)
}

pub(crate) async fn invitations(
	cx: &rpc::QueryCx,
	organization: &str,
) -> rpc::Result<Vec<PendingInvitation>> {
	let access = authorize::organization(
		cx.db(),
		cx.context(),
		organization,
		Permission::ManageMembers,
	)
	.await?;
	let organization_id = access.organization.id;
	let now = leptoskit_db::now();
	let rows = Invitation::query()
		.filter(|row| {
			row.organization_id.eq(organization_id)
				& row.accepted_at.is_null()
				& row.revoked_at.is_null()
				& row.expires_at.gt(now)
		})
		.order_by(|row| row.created_at.asc())
		.all(cx.db())
		.await?;
	Ok(rows.iter().map(pending_of).collect())
}

pub(crate) async fn revoke_invitation(
	cx: &rpc::MutationCx,
	organization: &str,
	invitation: Uuid,
) -> rpc::Result<()> {
	let access = authorize::organization(
		cx.tx(),
		cx.context(),
		organization,
		Permission::ManageMembers,
	)
	.await?;
	let organization_id = access.organization.id;
	let id: Id<Invitation> = Id::from_uuid(invitation);
	let row = Invitation::query()
		.filter(|row| {
			row.id.eq(id)
				& row.organization_id.eq(organization_id)
				& row.accepted_at.is_null()
				& row.revoked_at.is_null()
		})
		.first(cx.tx())
		.await?
		.ok_or_else(|| {
			rpc::Error::not_found(format!(
				"invitation_not_found: `{organization}` has no pending invitation {invitation}"
			))
		})?;
	access.require_to_give(row.role)?;
	Invitation::update()
		.filter(|row| row.id.eq(id))
		.set(|row| row.revoked_at, Some(leptoskit_db::now()))
		.run(cx.tx())
		.await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"invitation.revoke",
		Some(organization_id),
		None,
		Some(format!("{invitation} {}", row.email)),
		&client(cx.context()),
	)
	.await?;
	Ok(())
}

/// The member `account` of the organisation `access` names, or `NotFound`.
async fn member(
	db: &impl leptoskit_db::Read,
	access: &authorize::Access,
	account: Uuid,
) -> rpc::Result<Membership> {
	membership_of(db, access.organization.id, Id::from_uuid(account))
		.await?
		.ok_or_else(|| {
			rpc::Error::not_found(format!(
				"member_not_found: {account} isn't in `{}`",
				access.organization.slug
			))
		})
}

/// `Conflict` (`last_owner`) when `member` is the organisation's only owner.
async fn keep_an_owner(
	db: &impl leptoskit_db::Read,
	access: &authorize::Access,
	member: &Membership,
) -> rpc::Result<()> {
	if member.role != Role::Owner {
		return Ok(());
	}

	let organization_id = access.organization.id;
	let owners = Membership::query()
		.filter(|row| row.organization_id.eq(organization_id) & row.role.eq(Role::Owner))
		.count(db)
		.await?;

	if owners <= 1 {
		return Err(rpc::Error::conflict(format!(
			"last_owner: `{}` needs at least one owner; make someone else an owner first",
			access.organization.slug
		)));
	}

	Ok(())
}

pub(crate) async fn set_member_role(
	cx: &rpc::MutationCx,
	organization: &str,
	account: Uuid,
	role: Role,
) -> rpc::Result<()> {
	let access = authorize::organization(
		cx.tx(),
		cx.context(),
		organization,
		Permission::ManageMembers,
	)
	.await?;
	let target = member(cx.tx(), &access, account).await?;
	access.require_to_give(target.role)?;
	access.require_to_give(role)?;

	if target.role != role {
		if role != Role::Owner {
			keep_an_owner(cx.tx(), &access, &target).await?;
		}

		Membership::update()
			.filter(|row| row.id.eq(target.id))
			.set(|row| row.role, role)
			.run(cx.tx())
			.await?;
		audit::record(
			cx.tx(),
			&Actor::from(&access.caller),
			"member.set_role",
			Some(access.organization.id),
			None,
			Some(format!("{account} {} -> {role}", target.role)),
			&client(cx.context()),
		)
		.await?;
		revoke_invitations_beyond(cx, &access, target.account_id, Some(role)).await?;
	}

	Ok(())
}

/// Revokes the pending invitations `inviter` sent that `role`, their role
/// now (`None` once they have gone), can't give, each with its own audit
/// entry: an invitation is only as good as its inviter's authority.
async fn revoke_invitations_beyond(
	cx: &rpc::MutationCx,
	access: &authorize::Access,
	inviter: Id<Account>,
	role: Option<Role>,
) -> rpc::Result<()> {
	let organization_id = access.organization.id;
	let now = leptoskit_db::now();
	let pending = Invitation::query()
		.filter(|row| {
			row.organization_id.eq(organization_id)
				& row.invited_by.eq(inviter)
				& row.accepted_at.is_null()
				& row.revoked_at.is_null()
				& row.expires_at.gt(now)
		})
		.all(cx.tx())
		.await?;

	for row in pending
		.iter()
		.filter(|row| !role.is_some_and(|role| role.may_give(row.role)))
	{
		let id = row.id;
		Invitation::update()
			.filter(|row| row.id.eq(id))
			.set(|row| row.revoked_at, Some(now))
			.run(cx.tx())
			.await?;
		audit::record(
			cx.tx(),
			&Actor::from(&access.caller),
			"invitation.revoke",
			Some(organization_id),
			None,
			Some(format!("{id} {}", row.email)),
			&client(cx.context()),
		)
		.await?;
	}

	Ok(())
}

pub(crate) async fn remove_member(
	cx: &rpc::MutationCx,
	organization: &str,
	account: Uuid,
) -> rpc::Result<()> {
	let access = authorize::visible_organization(cx.tx(), cx.context(), organization).await?;
	let target = member(cx.tx(), &access, account).await?;
	// Anyone may leave; removing someone else is managing members.
	let leaving =
		matches!(access.caller, Caller::Person { account: me, .. } if me == target.account_id);

	if !leaving {
		access.require_to_give(target.role)?;
	}

	keep_an_owner(cx.tx(), &access, &target).await?;
	let organization_id = access.organization.id;
	let removed_account = target.account_id;
	target.destroy(cx.tx()).await?;
	// Their tokens stop with them.
	let now = leptoskit_db::now();
	ApiToken::update()
		.filter(|token| {
			token.organization_id.eq(organization_id)
				& token.created_by.eq(removed_account)
				& token.revoked_at.is_null()
		})
		.set(|token| token.revoked_at, Some(now))
		.run(cx.tx())
		.await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		if leaving {
			"member.leave"
		} else {
			"member.remove"
		},
		Some(organization_id),
		None,
		Some(account.to_string()),
		&client(cx.context()),
	)
	.await?;
	// Their invitations go with them.
	revoke_invitations_beyond(cx, &access, removed_account, None).await?;
	Ok(())
}
