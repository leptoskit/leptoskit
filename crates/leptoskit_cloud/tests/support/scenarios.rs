//! What the control plane must do, written once and run on SQLite
//! (`control.rs`) and Postgres (`postgres.rs`).

use leptoskit_auth::Account;
use leptoskit_cloud::tables::ApiToken;
use leptoskit_cloud::tables::AuditEvent;
use leptoskit_cloud::tables::Invitation;
use leptoskit_cloud::tables::Organization;
use leptoskit_db::Id;
use serde_json::Value;
use serde_json::json;

use super::App;
use super::As;

fn id<T>(text: &str) -> Id<T> {
	Id::parse(text).expect("an ID")
}

/// The first person's first calls, at once, make one personal organisation
/// named after their address; the next person whose address makes the same
/// slug gets the next one.
pub async fn a_personal_organization_is_made_once(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let calls = (0..8).map(|_| app.call("organizations", json!({}), As::Session(&ana)));

	for answer in futures::future::join_all(calls).await {
		answer.expect(200);
	}

	let account: Id<Account> = id(&app.account(As::Session(&ana)).await);
	let owned = Organization::query()
		.filter(|organization| organization.personal_owner.eq(Some(account)))
		.all(&app.system())
		.await
		.expect("read");
	assert_eq!(owned.len(), 1);
	assert_eq!(owned[0].slug, "ana");

	let whoami = app.call("whoami", json!({}), As::Session(&ana)).await;
	let whoami = whoami.expect(200).json();
	assert_eq!(whoami["email"], "ana@example.com");
	assert_eq!(whoami["via"], "web");
	assert_eq!(whoami["operator"], false);
	assert_eq!(whoami["token"], Value::Null);
	assert_eq!(whoami["organizations"].as_array().map(Vec::len), Some(1));
	assert_eq!(whoami["organizations"][0]["role"], "owner");
	assert_eq!(whoami["organizations"][0]["plan"], "hobby");

	let other = app.sign_in("ana@example.org").await;
	assert_eq!(app.personal(As::Session(&other)).await, "ana-2");
}

/// Nobody signed in reaches nothing, and neither does a credential that
/// isn't one.
pub async fn anonymous_callers_reach_nothing(app: &App) {
	for caller in [
		As::Nobody,
		As::Bearer(
			"lkc_000000000000_0000000000000000000000000000000000000000000000000000000000000000",
		),
		As::Bearer("lkc_nope"),
		As::Session("leptoskit_session=forged"),
	] {
		app.call("whoami", json!({}), caller)
			.await
			.refused(401, "sign_in_required");
		app.call("projects", json!({ "organization": "ana" }), caller)
			.await
			.refused(401, "sign_in_required");
	}

	// Something shaped like an access token that isn't one is turned away
	// by the OAuth provider's layer before it reaches an operation.
	let forged = As::Bearer("eyJhbGciOiJub25lIn0.e30.");
	let answer = app.call("whoami", json!({}), forged).await;
	assert_eq!(answer.status, 401, "{}", answer.text);
	assert!(answer.text.contains("invalid_token"), "{}", answer.text);
}

/// A token's value is shown once and stored as a hash; it lasts 90 days
/// unless asked otherwise, is touched at most every few minutes, and stops
/// at once when revoked or expired.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn tokens_are_shown_once_and_stop_when_revoked(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	app.project(As::Session(&ana), &org, "web").await;

	let created = app
		.call(
			"create_token",
			json!({ "organization": org, "name": "ci", "scopes": ["manage", "read", "read"] }),
			As::Session(&ana),
		)
		.await;
	let created = created.expect(200).json();
	let value = created["value"].as_str().expect("a value").to_owned();
	let prefix = created["token"]["prefix"]
		.as_str()
		.expect("a prefix")
		.to_owned();
	let secret = value.rsplit('_').next().expect("a secret").to_owned();
	assert_eq!(value, format!("lkc_{prefix}_{secret}"));
	assert_eq!(secret.len(), 64);
	assert_eq!(created["token"]["scopes"], json!(["read", "manage"]));

	let listed = app
		.call("tokens", json!({ "organization": org }), As::Session(&ana))
		.await;
	listed.expect(200);
	assert!(!listed.text.contains(&secret));
	assert_eq!(listed.json()[0].get("value"), None);

	let row = || {
		async {
			let prefix = prefix.clone();
			ApiToken::query()
				.filter(|token| token.prefix.eq(prefix))
				.one(&app.system())
				.await
				.expect("the token's row")
		}
	};
	let stored = row().await;
	assert_eq!(stored.secret_hash.len(), 32);
	assert_ne!(stored.secret_hash, secret.as_bytes());
	// The row's `created_at` is stamped as it is written, a moment after the
	// expiry is reckoned.
	let lifetime = stored.expires_at - stored.created_at;
	assert!(
		(lifetime - chrono::TimeDelta::days(90)).abs() < chrono::TimeDelta::seconds(1),
		"{lifetime:?}"
	);
	assert_eq!(stored.last_used_at, None);

	let whoami = app.call("whoami", json!({}), As::Bearer(&value)).await;
	let whoami = whoami.expect(200).json();
	assert_eq!(whoami["via"], "token");
	assert_eq!(whoami["email"], Value::Null);
	assert_eq!(whoami["token"]["organization"], org.as_str());
	assert_eq!(whoami["token"]["scopes"], json!(["read", "manage"]));
	assert_eq!(whoami["organizations"].as_array().map(Vec::len), Some(1));
	let used = row().await.last_used_at.expect("used");
	app.call(
		"projects",
		json!({ "organization": org }),
		As::Bearer(&value),
	)
	.await
	.expect(200);
	assert_eq!(
		row().await.last_used_at,
		Some(used),
		"touched again too soon"
	);

	// The wrong secret, or a value cut short, is nobody.
	let wrong = format!("lkc_{prefix}_{}", "0".repeat(64));
	for bad in [wrong.as_str(), &value[..value.len() - 1]] {
		app.call("whoami", json!({}), As::Bearer(bad))
			.await
			.refused(401, "sign_in_required");
	}

	for (args, code) in [
		(json!({ "scopes": [] }), "invalid_scopes"),
		(
			json!({ "scopes": ["read"], "expires_in_days": 10 }),
			"invalid_lifetime",
		),
		(json!({ "scopes": ["read"], "name": " " }), "invalid_name"),
	] {
		let mut body = json!({ "organization": org, "name": "ci" });
		for (key, value) in args.as_object().expect("an object") {
			body[key] = value.clone();
		}
		app.call("create_token", body, As::Session(&ana))
			.await
			.refused(400, code);
	}

	let year = app
		.call(
			"create_token",
			json!({
				"organization": org,
				"name": "yearly",
				"scopes": ["read"],
				"expires_in_days": 365,
			}),
			As::Session(&ana),
		)
		.await;
	year.expect(200);

	// Expiry stops a token, and hides it from the list.
	let (short, short_id) = app.token(As::Session(&ana), &org, &["read"], None).await;
	let short_id: Id<ApiToken> = id(&short_id);
	ApiToken::update()
		.filter(|token| token.id.eq(short_id))
		.set(
			|token| token.expires_at,
			leptoskit_db::now() - chrono::TimeDelta::seconds(1),
		)
		.run(&app.system())
		.await
		.expect("expire it");
	app.call("whoami", json!({}), As::Bearer(&short))
		.await
		.refused(401, "sign_in_required");
	let listed = app
		.call("tokens", json!({ "organization": org }), As::Session(&ana))
		.await;
	assert_eq!(listed.expect(200).json().as_array().map(Vec::len), Some(2));

	// Revoking stops it at the next request.
	let token = created["token"]["id"].as_str().expect("an id");
	app.call(
		"revoke_token",
		json!({ "organization": org, "token": token }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	app.call("whoami", json!({}), As::Bearer(&value))
		.await
		.refused(401, "sign_in_required");
	app.call(
		"revoke_token",
		json!({ "organization": org, "token": token }),
		As::Session(&ana),
	)
	.await
	.refused(404, "token_not_found");
}

/// A token gives no more than its maker has: the scopes their role grants.
/// A token makes no tokens, whatever its scopes. A demotion or leaving
/// reaches tokens at once.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn tokens_are_never_stronger_than_their_maker(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	app.project(As::Session(&ana), &org, "web").await;
	app.project(As::Session(&ana), &org, "mobile").await;
	let bea = app
		.join(As::Session(&ana), &org, "bea@example.com", "admin")
		.await;
	let dev = app
		.join(As::Session(&ana), &org, "dev@example.com", "developer")
		.await;

	// An admin can't give billing; a developer can't make tokens at all.
	app.call(
		"create_token",
		json!({ "organization": org, "name": "x", "scopes": ["billing"] }),
		As::Session(&bea),
	)
	.await
	.refused(403, "token_too_strong");
	app.call(
		"create_token",
		json!({ "organization": org, "name": "x", "scopes": ["read"] }),
		As::Session(&dev),
	)
	.await
	.refused(403, "insufficient_role");

	// No token makes a token, not even one with every scope its owner can
	// give: a token it made would outlive its own revocation.
	let (every, _) = app
		.token(
			As::Session(&ana),
			&org,
			&["read", "deploy", "manage", "publish", "billing", "admin"],
			None,
		)
		.await;
	app.call(
		"create_token",
		json!({ "organization": org, "name": "x", "scopes": ["read"] }),
		As::Bearer(&every),
	)
	.await
	.refused(403, "person_required");

	// A project-bound token sees its project, and no other: the others
	// don't exist for it.
	let (bound, _) = app
		.token(As::Session(&ana), &org, &["read", "manage"], Some("web"))
		.await;
	let projects = app
		.call(
			"projects",
			json!({ "organization": org }),
			As::Bearer(&bound),
		)
		.await;
	let slugs: Vec<Value> = projects
		.expect(200)
		.json()
		.as_array()
		.expect("a list")
		.iter()
		.map(|project| project["slug"].clone())
		.collect();
	assert_eq!(slugs, [json!("web")]);
	app.call(
		"project",
		json!({ "organization": org, "project": "web" }),
		As::Bearer(&bound),
	)
	.await
	.expect(200);
	let other = app
		.call(
			"project",
			json!({ "organization": org, "project": "mobile" }),
			As::Bearer(&bound),
		)
		.await;
	let missing = app
		.call(
			"project",
			json!({ "organization": org, "project": "nope" }),
			As::Bearer(&bound),
		)
		.await;
	other.refused(404, "project_not_found");
	missing.refused(404, "project_not_found");
	app.call(
		"create_environment",
		json!({ "organization": org, "project": "mobile", "name": "staging", "kind": "staging" }),
		As::Bearer(&bound),
	)
	.await
	.refused(404, "project_not_found");
	app.call(
		"create_environment",
		json!({ "organization": org, "project": "web", "name": "staging", "kind": "staging" }),
		As::Bearer(&bound),
	)
	.await
	.expect(200);
	app.call(
		"create_project",
		json!({ "organization": org, "slug": "more", "name": "More" }),
		As::Bearer(&bound),
	)
	.await
	.refused(403, "project_token");

	// Bea's admin token follows her role down, and out.
	let (beas, _) = app
		.token(As::Session(&bea), &org, &["admin", "manage"], None)
		.await;
	app.call(
		"create_project",
		json!({ "organization": org, "slug": "beas", "name": "Bea's" }),
		As::Bearer(&beas),
	)
	.await
	.expect(200);
	let bea_account = app.account(As::Session(&bea)).await;
	app.call(
		"set_member_role",
		json!({ "organization": org, "account": bea_account, "role": "viewer" }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	app.call(
		"create_project",
		json!({ "organization": org, "slug": "again", "name": "Again" }),
		As::Bearer(&beas),
	)
	.await
	.refused(403, "insufficient_role");
	app.call(
		"remove_member",
		json!({ "organization": org, "account": bea_account }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	app.call("whoami", json!({}), As::Bearer(&beas))
		.await
		.refused(401, "sign_in_required");
}

/// An invitation is mailed, works once for the address it was sent to,
/// and fails the same way unknown, revoked, expired, or used by someone
/// else.
#[expect(
	clippy::cognitive_complexity,
	clippy::too_many_lines,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn invitations_work_once_for_their_address(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let token = app
		.invite(As::Session(&ana), &org, "Bob@Example.com", "developer")
		.await;
	let mail = app.mailbox.latest_to("bob@example.com").expect("mailed");
	assert!(
		mail.text
			.contains(&format!("{}/invitations/{token}", app.origin))
	);
	assert!(mail.text.contains("ana@example.com invited you"));
	assert_eq!(token.len(), 64);

	let pending = app
		.call(
			"invitations",
			json!({ "organization": org }),
			As::Session(&ana),
		)
		.await;
	pending.expect(200);
	assert!(!pending.text.contains(&token));
	assert_eq!(pending.json()[0]["email"], "bob@example.com");
	assert_eq!(pending.json()[0]["role"], "developer");

	// Someone else can't use it.
	let carol = app.sign_in("carol@example.com").await;
	app.call(
		"accept_invitation",
		json!({ "token": token }),
		As::Session(&carol),
	)
	.await
	.refused(403, "wrong_email");

	// Bob can, and again, with the same answer.
	let bob = app.sign_in("bob@example.com").await;
	for _ in 0..2 {
		let accepted = app
			.call(
				"accept_invitation",
				json!({ "token": token }),
				As::Session(&bob),
			)
			.await;
		let accepted = accepted.expect(200).json();
		assert_eq!(accepted["slug"], org.as_str());
		assert_eq!(accepted["role"], "developer");
	}

	let members = app
		.call("members", json!({ "organization": org }), As::Session(&bob))
		.await;
	let members = members.expect(200).json();
	assert_eq!(members.as_array().map(Vec::len), Some(2));
	assert_eq!(members[1]["email"], "bob@example.com");
	let pending = app
		.call(
			"invitations",
			json!({ "organization": org }),
			As::Session(&ana),
		)
		.await;
	assert_eq!(pending.expect(200).json(), json!([]));

	// Used, unknown, revoked, and expired all read alike.
	let used = app
		.call(
			"accept_invitation",
			json!({ "token": token }),
			As::Session(&carol),
		)
		.await;
	let unknown = app
		.call(
			"accept_invitation",
			json!({ "token": "0".repeat(64) }),
			As::Session(&carol),
		)
		.await;
	let revoked_token = app
		.invite(As::Session(&ana), &org, "carol@example.com", "viewer")
		.await;
	let invitation = app
		.call(
			"invitations",
			json!({ "organization": org }),
			As::Session(&ana),
		)
		.await
		.json()[0]["id"]
		.clone();
	app.call(
		"revoke_invitation",
		json!({ "organization": org, "invitation": invitation }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	let revoked = app
		.call(
			"accept_invitation",
			json!({ "token": revoked_token }),
			As::Session(&carol),
		)
		.await;
	let expired_token = app
		.invite(As::Session(&ana), &org, "carol@example.com", "viewer")
		.await;
	Invitation::update()
		.filter(|invitation| {
			invitation.email.eq("carol@example.com") & invitation.revoked_at.is_null()
		})
		.set(|invitation| invitation.expires_at, leptoskit_db::now())
		.run(&app.system())
		.await
		.expect("expire it");
	let expired = app
		.call(
			"accept_invitation",
			json!({ "token": expired_token }),
			As::Session(&carol),
		)
		.await;

	for answer in [&used, &unknown, &revoked, &expired] {
		answer.refused(404, "invitation_invalid");
		assert_eq!(answer.text, used.text);
	}

	// A member invited again keeps their role when they accept; a developer
	// can't invite; an admin can't invite an owner; a token can't accept.
	let again = app
		.invite(As::Session(&ana), &org, "bob@example.com", "viewer")
		.await;
	let accepted = app
		.call(
			"accept_invitation",
			json!({ "token": again }),
			As::Session(&bob),
		)
		.await;
	assert_eq!(accepted.expect(200).json()["role"], "developer");
	app.call(
		"invite_member",
		json!({ "organization": org, "email": "dan@example.com", "role": "viewer" }),
		As::Session(&bob),
	)
	.await
	.refused(403, "insufficient_role");
	let eve = app
		.join(As::Session(&ana), &org, "eve@example.com", "admin")
		.await;
	app.call(
		"invite_member",
		json!({ "organization": org, "email": "dan@example.com", "role": "owner" }),
		As::Session(&eve),
	)
	.await
	.refused(403, "insufficient_role");
	app.call(
		"invite_member",
		json!({ "organization": org, "email": "not an address", "role": "viewer" }),
		As::Session(&ana),
	)
	.await
	.refused(400, "invalid_email");
	let (admin_token, _) = app.token(As::Session(&ana), &org, &["admin"], None).await;
	let dans = app
		.invite(As::Session(&ana), &org, "dan@example.com", "viewer")
		.await;
	app.call(
		"accept_invitation",
		json!({ "token": dans }),
		As::Bearer(&admin_token),
	)
	.await
	.refused(403, "person_required");
}

/// Owners stay: the last can't leave or step down. Only owners touch the
/// owner role, and anyone may leave.
pub async fn an_organization_keeps_an_owner(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let ana_account = app.account(As::Session(&ana)).await;

	app.call(
		"remove_member",
		json!({ "organization": org, "account": ana_account }),
		As::Session(&ana),
	)
	.await
	.refused(409, "last_owner");
	app.call(
		"set_member_role",
		json!({ "organization": org, "account": ana_account, "role": "admin" }),
		As::Session(&ana),
	)
	.await
	.refused(409, "last_owner");

	let bea = app
		.join(As::Session(&ana), &org, "bea@example.com", "admin")
		.await;
	let bea_account = app.account(As::Session(&bea)).await;
	let viv = app
		.join(As::Session(&ana), &org, "viv@example.com", "viewer")
		.await;
	let viv_account = app.account(As::Session(&viv)).await;

	// An admin can't make herself an owner, or touch one.
	app.call(
		"set_member_role",
		json!({ "organization": org, "account": bea_account, "role": "owner" }),
		As::Session(&bea),
	)
	.await
	.refused(403, "insufficient_role");
	app.call(
		"remove_member",
		json!({ "organization": org, "account": ana_account }),
		As::Session(&bea),
	)
	.await
	.refused(403, "insufficient_role");
	// A viewer can't remove anyone else.
	app.call(
		"remove_member",
		json!({ "organization": org, "account": bea_account }),
		As::Session(&viv),
	)
	.await
	.refused(403, "insufficient_role");
	// An admin manages the rest.
	app.call(
		"set_member_role",
		json!({ "organization": org, "account": viv_account, "role": "developer" }),
		As::Session(&bea),
	)
	.await
	.expect(200);
	let members = app
		.call("members", json!({ "organization": org }), As::Session(&bea))
		.await;
	assert_eq!(members.expect(200).json()[2]["role"], "developer");

	// With a second owner, the first may step down, and leave.
	app.call(
		"set_member_role",
		json!({ "organization": org, "account": bea_account, "role": "owner" }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	app.call(
		"remove_member",
		json!({ "organization": org, "account": ana_account }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	app.call("members", json!({ "organization": org }), As::Session(&ana))
		.await
		.refused(404, "organization_not_found");
	app.call(
		"remove_member",
		json!({ "organization": org, "account": bea_account }),
		As::Session(&bea),
	)
	.await
	.refused(409, "last_owner");

	// Anyone may leave.
	app.call(
		"remove_member",
		json!({ "organization": org, "account": viv_account }),
		As::Session(&viv),
	)
	.await
	.expect(200);
}

/// Two organisations with same-named projects can't see into each other:
/// by slug or by raw ID, the other's things don't exist.
pub async fn tenants_are_isolated(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let bob = app.sign_in("bob@example.com").await;
	let anas = app.personal(As::Session(&ana)).await;
	let bobs = app.personal(As::Session(&bob)).await;
	app.project(As::Session(&ana), &anas, "web").await;
	app.project(As::Session(&bob), &bobs, "web").await;
	let (ana_token, ana_token_id) = app
		.token(As::Session(&ana), &anas, &["admin", "manage"], None)
		.await;
	app.invite(As::Session(&ana), &anas, "carol@example.com", "viewer")
		.await;
	let invitation = app
		.call(
			"invitations",
			json!({ "organization": anas }),
			As::Session(&ana),
		)
		.await
		.json()[0]["id"]
		.clone();
	let ana_account = app.account(As::Session(&ana)).await;

	// Ana's organisation, to Bob, is the same as one that doesn't exist.
	for organization in [anas.as_str(), "nobody-here"] {
		for (operation, args) in [
			("members", json!({})),
			("projects", json!({})),
			("project", json!({ "project": "web" })),
			("environments", json!({ "project": "web" })),
			("tokens", json!({})),
			("invitations", json!({})),
			(
				"delete_project",
				json!({ "project": "web", "confirm": "web" }),
			),
			(
				"create_project",
				json!({ "slug": "sneaky", "name": "Sneaky" }),
			),
		] {
			let mut args = args;
			args["organization"] = json!(organization);
			let answer = app.call(operation, args, As::Session(&bob)).await;
			answer.refused(404, "organization_not_found");
			assert_eq!(
				answer.message(),
				format!(
					"organization_not_found: there is no organisation `{organization}` that you \
					 can see"
				)
			);
		}
	}

	// Ana's IDs, named in Bob's organisation, are nothing there.
	app.call(
		"revoke_token",
		json!({ "organization": bobs, "token": ana_token_id }),
		As::Session(&bob),
	)
	.await
	.refused(404, "token_not_found");
	app.call(
		"revoke_invitation",
		json!({ "organization": bobs, "invitation": invitation }),
		As::Session(&bob),
	)
	.await
	.refused(404, "invitation_not_found");
	for operation in ["set_member_role", "remove_member"] {
		app.call(
			operation,
			json!({ "organization": bobs, "account": ana_account, "role": "viewer" }),
			As::Session(&bob),
		)
		.await
		.refused(404, "member_not_found");
	}

	// Ana's token, pointed at Bob's organisation, finds nothing either.
	app.call(
		"project",
		json!({ "organization": bobs, "project": "web" }),
		As::Bearer(&ana_token),
	)
	.await
	.refused(404, "organization_not_found");

	// And each still sees their own.
	app.call(
		"project",
		json!({ "organization": anas, "project": "web" }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	let whoami = app.call("whoami", json!({}), As::Session(&bob)).await;
	assert_eq!(
		whoami.expect(200).json()["organizations"]
			.as_array()
			.map(Vec::len),
		Some(1)
	);
}

/// Projects come with `production`, report what was detected without
/// values, and an environment changes only from the revision it was read
/// at. Deleting needs the name typed back and the `admin` scope.
#[expect(
	clippy::cognitive_complexity,
	clippy::too_many_lines,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn projects_and_environments(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let created = app
		.call(
			"create_project",
			json!({
				"organization": org,
				"slug": "web",
				"name": "Web",
				"detected": {
					"targets": ["web", "server", "web"],
					"variables": ["DATABASE_URL", "API_KEY"],
				},
			}),
			As::Session(&ana),
		)
		.await;
	let created = created.expect(200).json();
	assert_eq!(created["project"]["region"], "eu-central");
	assert_eq!(created["environment"]["name"], "production");
	assert_eq!(created["environment"]["kind"], "production");
	assert_eq!(created["environment"]["scale_differs"], false);
	assert_eq!(created["detected"]["targets"], json!(["server", "web"]));
	assert_eq!(
		created["detected"]["variables"],
		json!(["API_KEY", "DATABASE_URL"])
	);

	for (args, status, code) in [
		(json!({ "slug": "web", "name": "Again" }), 409, "slug_taken"),
		(
			json!({ "slug": "settings", "name": "S" }),
			400,
			"invalid_slug",
		),
		(
			json!({ "slug": "Bad Slug", "name": "S" }),
			400,
			"invalid_slug",
		),
		(
			json!({ "slug": "ok-slug", "name": "S", "region": "Mars!" }),
			400,
			"invalid_region",
		),
		(
			json!({ "slug": "ok-slug", "name": "S", "detected": { "variables": ["KEY=hunter2"] } }),
			400,
			"invalid_detected",
		),
	] {
		let mut args = args;
		args["organization"] = json!(org);
		let answer = app.call("create_project", args, As::Session(&ana)).await;
		answer.refused(status, code);
		assert!(!answer.text.contains("hunter2"));
	}

	let staging = app
		.call(
			"create_environment",
			json!({ "organization": org, "project": "web", "name": "staging", "kind": "staging" }),
			As::Session(&ana),
		)
		.await;
	let revision = staging.expect(200).json()["revision"]
		.as_i64()
		.expect("a revision");
	for (name, kind, code) in [
		("production", "production", "invalid_environment"),
		("other", "production", "invalid_environment"),
		("releases", "staging", "invalid_slug"),
	] {
		app.call(
			"create_environment",
			json!({ "organization": org, "project": "web", "name": name, "kind": kind }),
			As::Session(&ana),
		)
		.await
		.refused(400, code);
	}
	app.call(
		"create_environment",
		json!({ "organization": org, "project": "web", "name": "staging", "kind": "preview" }),
		As::Session(&ana),
	)
	.await
	.refused(409, "name_taken");

	let scale = json!({ "app": { "size": "s", "min": 1, "max": 3, "sleep": "never" } });
	let update = |revision: i64, scale: Value| {
		app.call(
			"update_environment",
			json!({
				"organization": org,
				"project": "web",
				"environment": "staging",
				"revision": revision,
				"scale": scale,
			}),
			As::Session(&ana),
		)
	};
	let updated = update(revision, scale.clone()).await;
	let updated = updated.expect(200).json();
	assert_eq!(updated["revision"].as_i64(), Some(revision + 1));
	assert_eq!(updated["scale_differs"], true);
	assert_eq!(updated["scale"]["app"]["max"], 3);
	update(revision, scale.clone())
		.await
		.refused(409, "stale_revision");
	update(
		revision + 1,
		json!({ "app": { "size": "s", "min": 3, "max": 1, "sleep": "never" } }),
	)
	.await
	.refused(400, "invalid_scale");

	let environments = app
		.call(
			"environments",
			json!({ "organization": org, "project": "web" }),
			As::Session(&ana),
		)
		.await;
	let names: Vec<Value> = environments
		.expect(200)
		.json()
		.as_array()
		.expect("a list")
		.iter()
		.map(|environment| environment["name"].clone())
		.collect();
	assert_eq!(names, [json!("production"), json!("staging")]);

	// Deleting: the name typed back, the `admin` scope, never production.
	let (manage, _) = app.token(As::Session(&ana), &org, &["manage"], None).await;
	let (admin, _) = app.token(As::Session(&ana), &org, &["admin"], None).await;
	app.call(
		"delete_environment",
		json!({ "organization": org, "project": "web", "environment": "staging", "confirm": "staging" }),
		As::Bearer(&manage),
	)
	.await
	.refused(403, "insufficient_scope");
	app.call(
		"delete_environment",
		json!({ "organization": org, "project": "web", "environment": "staging", "confirm": "stag" }),
		As::Bearer(&admin),
	)
	.await
	.refused(400, "confirmation_mismatch");
	app.call(
		"delete_environment",
		json!({ "organization": org, "project": "web", "environment": "production", "confirm": "production" }),
		As::Bearer(&admin),
	)
	.await
	.refused(403, "production");
	app.call(
		"delete_environment",
		json!({ "organization": org, "project": "web", "environment": "staging", "confirm": "staging" }),
		As::Bearer(&admin),
	)
	.await
	.expect(200);
	app.call(
		"delete_project",
		json!({ "organization": org, "project": "web", "confirm": "Web" }),
		As::Session(&ana),
	)
	.await
	.refused(400, "confirmation_mismatch");
	app.call(
		"delete_project",
		json!({ "organization": org, "project": "web", "confirm": "web" }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	app.call(
		"project",
		json!({ "organization": org, "project": "web" }),
		As::Session(&ana),
	)
	.await
	.refused(404, "project_not_found");
}

/// Every change is in the audit log with who made it and from where, and
/// no entry holds a secret.
pub async fn the_audit_log_names_the_actor_and_no_secrets(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let (token, token_id) = app
		.token(As::Session(&ana), &org, &["admin", "manage"], None)
		.await;
	app.call(
		"create_project",
		json!({ "organization": org, "slug": "web", "name": "Web" }),
		As::Bearer(&token),
	)
	.await
	.expect(200);
	let invitation = app
		.invite(As::Session(&ana), &org, "bob@example.com", "viewer")
		.await;

	let entries = AuditEvent::query()
		.order_by(|entry| entry.created_at.asc())
		.all(&app.system())
		.await
		.expect("read");
	let actions: Vec<&str> = entries.iter().map(|entry| entry.action.as_str()).collect();
	assert_eq!(
		actions,
		[
			"organization.create_personal",
			"token.create",
			"project.create",
			"invitation.create",
		]
	);
	let ana_account: Id<Account> = id(&app.account(As::Session(&ana)).await);

	for entry in &entries {
		assert_eq!(entry.account_id, Some(ana_account));
		assert!(entry.organization_id.is_some());
	}

	let by_token = &entries[2];
	assert_eq!(by_token.token_id, Some(id(&token_id)));
	assert!(
		by_token.client.starts_with("token: test-1 (lkc_"),
		"{}",
		by_token.client
	);
	assert!(
		entries[1].client.starts_with("web: "),
		"{}",
		entries[1].client
	);
	let secret = token.rsplit('_').next().expect("a secret");

	for entry in &entries {
		let text = format!("{} {} {:?}", entry.client, entry.action, entry.target);
		assert!(!text.contains(secret), "{text}");
		assert!(!text.contains(&invitation), "{text}");
	}
}

/// The CLI signs in with the device flow, and its access token is the
/// person; another app's token is nobody.
pub async fn the_cli_signs_in_with_the_device_flow(app: &App) {
	let discovery = app
		.get(leptoskit_cloud::server::DISCOVERY_PATH, As::Nobody)
		.await;
	let discovery = discovery.expect(200).json();
	let client_id = discovery["cli_client_id"]
		.as_str()
		.expect("a client")
		.to_owned();
	assert_eq!(client_id, app.control.cli_client_id());
	let scope = discovery["scope"].as_str().expect("scopes").to_owned();

	let ana = app.sign_in("ana@example.com").await;
	let tokens = app.device_flow(&client_id, &scope, &ana).await;
	let access = tokens["access_token"].as_str().expect("an access token");
	assert!(tokens["refresh_token"].is_string(), "{tokens}");

	let whoami = app.call("whoami", json!({}), As::Bearer(access)).await;
	let whoami = whoami.expect(200).json();
	assert_eq!(whoami["via"], "cli");
	assert_eq!(whoami["email"], "ana@example.com");
	let org = whoami["organizations"][0]["slug"].as_str().expect("a slug");
	app.project(As::Bearer(access), org, "from-cli").await;
	let created = AuditEvent::query()
		.filter(|entry| entry.action.eq("project.create"))
		.one(&app.system())
		.await
		.expect("an entry");
	assert_eq!(created.client, "cli");

	// Another first-party device app's token is nobody here.
	let (other, _) = app
		.control
		.idp()
		.register_client(leptoskit_idp::NewClientSpec {
			name: "Something else".to_owned(),
			redirect_uris: Vec::new(),
			post_logout_redirect_uris: Vec::new(),
			scopes: vec!["openid".to_owned(), "cloud".to_owned()],
			auth_method: leptoskit_idp::ClientAuth::None,
			grant_types: vec![leptoskit_idp::GrantType::DeviceCode],
			jwks: None,
			id_token_alg: leptoskit_idp::SigningAlgorithm::Es256,
			first_party: true,
			introspection: false,
			dpop_bound: false,
		})
		.await
		.expect("registered");
	let tokens = app
		.device_flow(&other.client_id, "openid cloud", &ana)
		.await;
	let access = tokens["access_token"].as_str().expect("an access token");
	app.call("whoami", json!({}), As::Bearer(access))
		.await
		.refused(401, "sign_in_required");
}

/// An operator granted on the command line is one, by `whoami`.
pub async fn operators_are_granted_on_the_command_line(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	assert!(matches!(
		leptoskit_cloud::server::grant_operator(&app.db, "nobody@example.com").await,
		Err(leptoskit_cloud::server::OperatorError::UnknownEmail(_))
	));
	leptoskit_cloud::server::grant_operator(&app.db, "ANA@example.com")
		.await
		.expect("granted");
	leptoskit_cloud::server::grant_operator(&app.db, "ana@example.com")
		.await
		.expect("granted twice is once");
	assert_eq!(
		leptoskit_cloud::server::operators(&app.db)
			.await
			.expect("listed"),
		["ana@example.com"]
	);
	let whoami = app.call("whoami", json!({}), As::Session(&ana)).await;
	assert_eq!(whoami.expect(200).json()["operator"], true);
	assert!(
		leptoskit_cloud::server::revoke_operator(&app.db, "ana@example.com")
			.await
			.expect("revoked")
	);
	let whoami = app.call("whoami", json!({}), As::Session(&ana)).await;
	assert_eq!(whoami.expect(200).json()["operator"], false);
}

/// An API token, whatever its scopes, can't change who is in an
/// organisation or which tokens act in it: the review's escalation, where
/// an `admin` token invites someone as an owner, stops at the invitation.
pub async fn tokens_never_grant_membership(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let bob = app
		.join(As::Session(&ana), &org, "bob@example.com", "viewer")
		.await;
	let bob_account = app.account(As::Session(&bob)).await;
	let ana_account = app.account(As::Session(&ana)).await;
	app.invite(As::Session(&ana), &org, "pat@example.com", "viewer")
		.await;
	let invitation = app
		.call(
			"invitations",
			json!({ "organization": org }),
			As::Session(&ana),
		)
		.await
		.json()[0]["id"]
		.clone();
	let (_, other_token) = app.token(As::Session(&ana), &org, &["read"], None).await;
	let (admin, _) = app
		.token(
			As::Session(&ana),
			&org,
			&["admin", "manage", "read", "deploy", "publish", "billing"],
			None,
		)
		.await;

	for (operation, args) in [
		(
			"invite_member",
			json!({ "email": "mallory@example.com", "role": "owner" }),
		),
		(
			"invite_member",
			json!({ "email": "mallory@example.com", "role": "viewer" }),
		),
		("invitations", json!({})),
		("revoke_invitation", json!({ "invitation": invitation })),
		(
			"set_member_role",
			json!({ "account": bob_account, "role": "admin" }),
		),
		("remove_member", json!({ "account": bob_account })),
		// Not even the person it acts for: a token doesn't leave.
		("remove_member", json!({ "account": ana_account })),
		("tokens", json!({})),
		(
			"create_token",
			json!({ "name": "child", "scopes": ["read"] }),
		),
		("revoke_token", json!({ "token": other_token })),
	] {
		let mut args = args;
		args["organization"] = json!(org);
		app.call(operation, args, As::Bearer(&admin))
			.await
			.refused(403, "person_required");
	}

	// Nothing changed: no one new was invited, and everyone is where they were.
	assert!(app.mailbox.latest_to("mallory@example.com").is_none());
	let members = app
		.call(
			"members",
			json!({ "organization": org }),
			As::Bearer(&admin),
		)
		.await;
	assert_eq!(members.expect(200).json().as_array().map(Vec::len), Some(2));
	let pending = app
		.call(
			"invitations",
			json!({ "organization": org }),
			As::Session(&ana),
		)
		.await;
	assert_eq!(pending.expect(200).json().as_array().map(Vec::len), Some(1));
}

/// An invitation is only as good as its inviter's authority when it is
/// accepted: one whose inviter has left, or been demoted below the role it
/// offers, fails like an unknown one.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn an_invitation_needs_its_inviters_authority(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let bea = app
		.join(As::Session(&ana), &org, "bea@example.com", "admin")
		.await;
	let bea_account = app.account(As::Session(&bea)).await;

	// Bea, an admin, invites Carol; Ana demotes Bea to developer, who can't
	// invite anyone. Carol's link no longer works, and isn't listed.
	let carols = app
		.invite(As::Session(&bea), &org, "carol@example.com", "developer")
		.await;
	app.call(
		"set_member_role",
		json!({ "organization": org, "account": bea_account, "role": "developer" }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	let carol = app.sign_in("carol@example.com").await;
	let unknown = app
		.call(
			"accept_invitation",
			json!({ "token": "0".repeat(64) }),
			As::Session(&carol),
		)
		.await;
	unknown.refused(404, "invitation_invalid");
	let demoted = app
		.call(
			"accept_invitation",
			json!({ "token": carols }),
			As::Session(&carol),
		)
		.await;
	assert_eq!(demoted.status, unknown.status, "{}", demoted.text);
	assert_eq!(demoted.text, unknown.text);
	let pending = app
		.call(
			"invitations",
			json!({ "organization": org }),
			As::Session(&ana),
		)
		.await;
	assert_eq!(pending.expect(200).json(), json!([]));
	let revoked = AuditEvent::query()
		.filter(|entry| entry.action.eq("invitation.revoke"))
		.all(&app.system())
		.await
		.expect("read");
	assert_eq!(revoked.len(), 1);
	assert!(
		revoked[0]
			.target
			.as_deref()
			.is_some_and(|target| target.contains("carol@example.com")),
		"{revoked:?}"
	);

	// An owner demoted to admin can no longer give the owner role, though
	// the admin invitations she sent still stand.
	app.call(
		"set_member_role",
		json!({ "organization": org, "account": bea_account, "role": "owner" }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	let dans = app
		.invite(As::Session(&bea), &org, "dan@example.com", "owner")
		.await;
	let eves = app
		.invite(As::Session(&bea), &org, "eve@example.com", "admin")
		.await;
	app.call(
		"set_member_role",
		json!({ "organization": org, "account": bea_account, "role": "admin" }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	let dan = app.sign_in("dan@example.com").await;
	app.call(
		"accept_invitation",
		json!({ "token": dans }),
		As::Session(&dan),
	)
	.await
	.refused(404, "invitation_invalid");
	let eve = app.sign_in("eve@example.com").await;
	let accepted = app
		.call(
			"accept_invitation",
			json!({ "token": eves }),
			As::Session(&eve),
		)
		.await;
	assert_eq!(accepted.expect(200).json()["role"], "admin");

	// However the inviter's role changes, acceptance checks it again itself:
	// here it changes underneath, with nothing else told.
	let fays = app
		.invite(As::Session(&bea), &org, "fay@example.com", "viewer")
		.await;
	let bea_id: Id<Account> = id(&bea_account);
	leptoskit_cloud::tables::Membership::update()
		.filter(|member| {
			member.account_id.eq(bea_id) & member.role.eq(leptoskit_cloud::access::Role::Admin)
		})
		.set(|member| member.role, leptoskit_cloud::access::Role::Billing)
		.run(&app.system())
		.await
		.expect("demote her underneath");
	let fay = app.sign_in("fay@example.com").await;
	app.call(
		"accept_invitation",
		json!({ "token": fays }),
		As::Session(&fay),
	)
	.await
	.refused(404, "invitation_invalid");

	// Someone removed takes their invitations with them.
	let gus = app
		.join(As::Session(&ana), &org, "gus@example.com", "admin")
		.await;
	let gus_account = app.account(As::Session(&gus)).await;
	let hals = app
		.invite(As::Session(&gus), &org, "hal@example.com", "viewer")
		.await;
	app.call(
		"remove_member",
		json!({ "organization": org, "account": gus_account }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	let hal = app.sign_in("hal@example.com").await;
	app.call(
		"accept_invitation",
		json!({ "token": hals }),
		As::Session(&hal),
	)
	.await
	.refused(404, "invitation_invalid");
}

/// Inviting an address again replaces its pending invitations, and needs
/// the authority to revoke each one: an admin can't cancel an owner's
/// invitation by re-inviting the address as a viewer. Each replaced
/// invitation has its own audit entry.
pub async fn replacing_an_invitation_needs_authority_over_it(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let bea = app
		.join(As::Session(&ana), &org, "bea@example.com", "admin")
		.await;
	let robs = app
		.invite(As::Session(&ana), &org, "rob@example.com", "owner")
		.await;
	app.call(
		"invite_member",
		json!({ "organization": org, "email": "rob@example.com", "role": "viewer" }),
		As::Session(&bea),
	)
	.await
	.refused(403, "insufficient_role");
	let rob = app.sign_in("rob@example.com").await;
	let accepted = app
		.call(
			"accept_invitation",
			json!({ "token": robs }),
			As::Session(&rob),
		)
		.await;
	assert_eq!(accepted.expect(200).json()["role"], "owner");

	// An owner replaces as she likes; the old link stops, and the log says
	// which invitation went and what replaced it.
	let first = app
		.invite(As::Session(&ana), &org, "sue@example.com", "viewer")
		.await;
	let pending = app
		.call(
			"invitations",
			json!({ "organization": org }),
			As::Session(&ana),
		)
		.await
		.json();
	let first_id = pending[0]["id"].as_str().expect("an id").to_owned();
	let second = app
		.invite(As::Session(&ana), &org, "sue@example.com", "developer")
		.await;
	let pending = app
		.call(
			"invitations",
			json!({ "organization": org }),
			As::Session(&ana),
		)
		.await
		.json();
	assert_eq!(pending.as_array().map(Vec::len), Some(1));
	let second_id = pending[0]["id"].as_str().expect("an id").to_owned();
	let revoked = AuditEvent::query()
		.filter(|entry| entry.action.eq("invitation.revoke"))
		.all(&app.system())
		.await
		.expect("read");
	assert_eq!(revoked.len(), 1, "{revoked:?}");
	let target = revoked[0].target.clone().unwrap_or_default();
	assert!(target.contains(&first_id), "{target}");
	assert!(target.contains(&second_id), "{target}");
	let sue = app.sign_in("sue@example.com").await;
	app.call(
		"accept_invitation",
		json!({ "token": first }),
		As::Session(&sue),
	)
	.await
	.refused(404, "invitation_invalid");
	let accepted = app
		.call(
			"accept_invitation",
			json!({ "token": second }),
			As::Session(&sue),
		)
		.await;
	assert_eq!(accepted.expect(200).json()["role"], "developer");
}

/// Nobody can stop someone else's first sign-in by taking the slugs their
/// personal organisation would get: after a few friendly names, it gets a
/// random suffix nobody can take in advance.
pub async fn personal_organizations_cannot_be_squatted(app: &App) {
	let mallory = app.sign_in("mallory@example.com").await;
	let mut taken = vec!["personal".to_owned(), "ana".to_owned()];
	taken.extend((2..=50).map(|n| format!("personal-{n}")));
	taken.extend((2..=50).map(|n| format!("ana-{n}")));

	for slug in &taken {
		app.call(
			"create_organization",
			json!({ "slug": slug, "name": "Squatted" }),
			As::Session(&mallory),
		)
		.await
		.expect(200);
	}

	// `jo` is too short for a slug, so Jo's starts from `personal`.
	for (email, base) in [("jo@example.com", "personal"), ("ana@example.com", "ana")] {
		let cookie = app.sign_in(email).await;
		let whoami = app.call("whoami", json!({}), As::Session(&cookie)).await;
		whoami.expect(200);
		let slug = app.personal(As::Session(&cookie)).await;
		let suffix = slug
			.strip_prefix(&format!("{base}-"))
			.unwrap_or_else(|| panic!("{slug} starts with {base}-"));
		assert_eq!(suffix.len(), 10, "{slug}");
		assert!(
			suffix
				.bytes()
				.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
			"{slug}"
		);
		assert!(!taken.contains(&slug));
	}
}

/// A token's value is never stored, even when it is made with an
/// idempotency key: no table holds it, the RPC's receipts included. A
/// retry of the same call is refused with the first token named, not
/// answered with a second token that looks like the first.
pub async fn token_secrets_are_never_stored(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let args = json!({ "organization": org, "name": "ci", "scopes": ["read"] });
	let created = app
		.call_with_key("create_token", args.clone(), As::Session(&ana), "retry-me")
		.await;
	let created = created.expect(200).json();
	let value = created["value"].as_str().expect("a value").to_owned();
	let prefix = created["token"]["prefix"]
		.as_str()
		.expect("a prefix")
		.to_owned();
	let secret = value.rsplit('_').next().expect("a secret").to_owned();

	// The scan finds what is stored, such as the prefix …
	assert_ne!(app.find_in_every_table(&prefix).await, Vec::<String>::new());
	// … and never the secret.
	let found = app.find_in_every_table(&secret).await;
	assert!(found.is_empty(), "the token's secret is stored: {found:?}");

	// The same call again, as a client retries one whose answer it lost.
	let again = app
		.call_with_key("create_token", args, As::Session(&ana), "retry-me")
		.await;
	again.refused(409, "token_name_taken");
	assert!(again.text.contains(&prefix), "{}", again.text);
	assert!(!again.text.contains(&secret), "{}", again.text);
	let listed = app
		.call("tokens", json!({ "organization": org }), As::Session(&ana))
		.await;
	assert_eq!(listed.expect(200).json().as_array().map(Vec::len), Some(1));

	// Racing calls with one name make one token.
	let racing = (0..8).map(|_| {
		app.call(
			"create_token",
			json!({ "organization": org, "name": "deploy", "scopes": ["read"] }),
			As::Session(&ana),
		)
	});
	let answers = futures::future::join_all(racing).await;
	let made = answers.iter().filter(|answer| answer.status == 200).count();
	assert_eq!(made, 1, "{answers:?}");
	for answer in answers.iter().filter(|answer| answer.status != 200) {
		answer.refused(409, "token_name_taken");
	}
}

/// Whether `answer` refused with `code`, or because the server was too busy
/// to finish: a racing call may run out of retries before it sees what the
/// winner wrote, and that is a refusal too.
fn refused_or_busy(answer: &super::Answer, code: &str) -> bool {
	answer.status == 409
		&& (answer.message().starts_with(code) || answer.message().contains("too busy to finish"))
}

/// Two owners changing each other at once can't leave the organisation
/// without an owner: one change wins, the other sees one owner left.
pub async fn the_last_owner_stays_under_concurrent_changes(app: &App) {
	for round in 0..3 {
		let ana = app.sign_in(&format!("ana{round}@example.com")).await;
		let org = app.personal(As::Session(&ana)).await;
		let bea = app
			.join(
				As::Session(&ana),
				&org,
				&format!("bea{round}@example.com"),
				"admin",
			)
			.await;
		let ana_account = app.account(As::Session(&ana)).await;
		let bea_account = app.account(As::Session(&bea)).await;
		app.call(
			"set_member_role",
			json!({ "organization": org, "account": bea_account, "role": "owner" }),
			As::Session(&ana),
		)
		.await
		.expect(200);

		// Each demotes, or removes, the other, at the same moment.
		let removing = round == 1;
		let change = |account: &str| {
			if removing {
				json!({ "organization": org, "account": account })
			} else {
				json!({ "organization": org, "account": account, "role": "admin" })
			}
		};
		let operation = if removing {
			"remove_member"
		} else {
			"set_member_role"
		};
		let ana_call = app.call(operation, change(&bea_account), As::Session(&ana));
		let bea_call = app.call(operation, change(&ana_account), As::Session(&bea));
		let (first, second) = futures::join!(ana_call, bea_call);
		let answers = [&first, &second];
		assert!(
			answers.iter().any(|answer| answer.status == 200),
			"{answers:?}"
		);

		// Whoever lost is no longer an owner, or no longer a member, by the
		// time their change runs; or, had both read two owners, theirs
		// would leave none.
		for answer in answers.iter().filter(|answer| answer.status != 200) {
			let lost = (answer.status == 403 && answer.message().starts_with("insufficient_role"))
				|| (answer.status == 404 && answer.message().starts_with("organization_not_found"))
				|| refused_or_busy(answer, "last_owner");
			assert!(lost, "{answer:?}");
		}

		let organization = Organization::query()
			.filter(|row| row.slug.eq(org.clone()))
			.one(&app.system())
			.await
			.expect("the organisation");
		let organization_id = organization.id;
		let owners = leptoskit_cloud::tables::Membership::query()
			.filter(|member| {
				member.organization_id.eq(organization_id)
					& member.role.eq(leptoskit_cloud::access::Role::Owner)
			})
			.count(&app.system())
			.await
			.expect("count");
		assert_eq!(owners, 1, "round {round}: {answers:?}");
	}
}

/// Racing creates can't take an organisation or project past its caps:
/// with one place left, one of eight calls gets it.
pub async fn caps_hold_under_concurrent_creates(app: &App) {
	use leptoskit_cloud::access::TokenScope;
	use leptoskit_cloud::projects::EnvironmentKind;
	use leptoskit_cloud::projects::EnvironmentScale;
	use leptoskit_cloud::tables::Environment;
	use leptoskit_cloud::tables::NewApiToken;
	use leptoskit_cloud::tables::NewEnvironment;
	use leptoskit_cloud::tables::NewProject;
	use leptoskit_cloud::tables::Project;

	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	app.project(As::Session(&ana), &org, "web").await;
	let system = app.system();
	let account: Id<Account> = id(&app.account(As::Session(&ana)).await);
	let organization = Organization::query()
		.filter(|row| row.slug.eq(org.clone()))
		.one(&system)
		.await
		.expect("the organisation");
	let web = Project::query()
		.filter(|row| row.slug.eq("web"))
		.one(&system)
		.await
		.expect("web");

	// One place left of each: 200 projects, 50 environments, 200 tokens.
	for n in 0..198 {
		NewProject {
			organization_id: organization.id,
			slug: format!("filler-{n}"),
			name: "Filler".to_owned(),
			region: "eu-central".to_owned(),
			detected_targets: leptoskit_db::Json(Vec::new()),
			detected_variables: leptoskit_db::Json(Vec::new()),
			created_by: account,
		}
		.insert(&system)
		.await
		.expect("a filler project");
	}

	for n in 0..48 {
		NewEnvironment {
			project_id: web.id,
			name: format!("filler-{n}"),
			kind: EnvironmentKind::Staging,
			scale: leptoskit_db::Json(EnvironmentScale::sleeping()),
			scale_differs: false,
		}
		.insert(&system)
		.await
		.expect("a filler environment");
	}

	for n in 0..199 {
		NewApiToken {
			organization_id: organization.id,
			project_id: None,
			name: format!("filler-{n}"),
			scopes: leptoskit_db::Json(vec![TokenScope::Read]),
			prefix: format!("{n:012x}"),
			secret_hash: vec![0; 32],
			expires_at: leptoskit_db::now() + chrono::TimeDelta::days(30),
			last_used_at: None,
			revoked_at: None,
			created_by: account,
		}
		.insert(&system)
		.await
		.expect("a filler token");
	}

	for (operation, code, args) in [
		(
			"create_project",
			"too_many_projects",
			(0..8)
				.map(
					|n| json!({ "organization": org, "slug": format!("race-{n}"), "name": "Race" }),
				)
				.collect::<Vec<_>>(),
		),
		(
			"create_environment",
			"too_many_environments",
			(0..8)
				.map(|n| {
					json!({
						"organization": org,
						"project": "web",
						"name": format!("race-{n}"),
						"kind": "preview",
					})
				})
				.collect(),
		),
		(
			"create_token",
			"too_many_tokens",
			(0..8)
				.map(
					|n| json!({ "organization": org, "name": format!("race-{n}"), "scopes": ["read"] }),
				)
				.collect(),
		),
	] {
		let calls = args
			.into_iter()
			.map(|args| app.call(operation, args, As::Session(&ana)));
		let answers = futures::future::join_all(calls).await;
		let made = answers.iter().filter(|answer| answer.status == 200).count();
		assert_eq!(made, 1, "{operation}: {answers:?}");

		for answer in answers.iter().filter(|answer| answer.status != 200) {
			assert!(refused_or_busy(answer, code), "{operation}: {answer:?}");
		}
	}

	let projects = Project::query()
		.filter(|row| row.organization_id.eq(organization.id))
		.count(&system)
		.await
		.expect("count");
	let web_id = web.id;
	let environments = Environment::query()
		.filter(|row| row.project_id.eq(web_id))
		.count(&system)
		.await
		.expect("count");
	let organization_id = organization.id;
	let tokens = ApiToken::query()
		.filter(|row| row.organization_id.eq(organization_id) & row.revoked_at.is_null())
		.count(&system)
		.await
		.expect("count");
	assert_eq!((projects, environments, tokens), (200, 50, 200));
}

/// A token can do only what its maker's role allows now: demoting them
/// narrows it at once, and removing them stops it.
pub async fn a_members_tokens_follow_their_role(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	app.project(As::Session(&ana), &org, "web").await;
	let bea = app
		.join(As::Session(&ana), &org, "bea@example.com", "admin")
		.await;
	let bea_account = app.account(As::Session(&bea)).await;
	let (token, token_id) = app
		.token(As::Session(&bea), &org, &["admin", "manage", "read"], None)
		.await;
	let environment = |name: &str| json!({ "organization": org, "project": "web", "name": name, "kind": "preview" });
	let delete = |name: &str| json!({ "organization": org, "project": "web", "environment": name, "confirm": name });
	let demote = |role: &str| {
		app.call(
			"set_member_role",
			json!({ "organization": org, "account": bea_account, "role": role }),
			As::Session(&ana),
		)
	};

	// As an admin's, it creates and deletes.
	for name in ["one", "two"] {
		app.call("create_environment", environment(name), As::Bearer(&token))
			.await
			.expect(200);
	}
	app.call("delete_environment", delete("one"), As::Bearer(&token))
		.await
		.expect(200);

	// As a developer's, it creates but no longer deletes.
	demote("developer").await.expect(200);
	app.call("delete_environment", delete("two"), As::Bearer(&token))
		.await
		.refused(403, "insufficient_role");
	app.call(
		"create_environment",
		environment("three"),
		As::Bearer(&token),
	)
	.await
	.expect(200);

	// As a viewer's, it only reads.
	demote("viewer").await.expect(200);
	app.call(
		"create_environment",
		environment("four"),
		As::Bearer(&token),
	)
	.await
	.refused(403, "insufficient_role");
	app.call(
		"project",
		json!({ "organization": org, "project": "web" }),
		As::Bearer(&token),
	)
	.await
	.expect(200);

	// Promoted again, it does again what its scopes and her role allow.
	demote("admin").await.expect(200);
	app.call("delete_environment", delete("two"), As::Bearer(&token))
		.await
		.expect(200);

	// Removed, she takes it with her: refused, and revoked for good.
	app.call(
		"remove_member",
		json!({ "organization": org, "account": bea_account }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	app.call("whoami", json!({}), As::Bearer(&token))
		.await
		.refused(401, "sign_in_required");
	let token_id: Id<ApiToken> = id(&token_id);
	let row = ApiToken::query()
		.filter(|row| row.id.eq(token_id))
		.one(&app.system())
		.await
		.expect("the token");
	assert!(row.revoked_at.is_some());
}
