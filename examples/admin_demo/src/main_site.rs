//! The main site: accounts, sign-in, and the identity provider the admin
//! signs in through. Its own requests feed the health tab.

use std::time::Duration;

use anyhow::Result;
use axum::Router;
use axum::extract::Query;
use axum::response::Html;
use axum::routing::get;
use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthContext as _;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::OnSignUp;
use leptoskit_auth::Profile;
use leptoskit_auth::Result as AuthResult;
use leptoskit_auth::Roles;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Tx;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_deploy_console::control::Store;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::ClientCommand;
use leptoskit_idp::ClientCommandOutput;
use leptoskit_idp::Idp;
use leptoskit_idp::NewClientSpec;
use leptoskit_idp::SigningAlgorithm;

use crate::ADMIN;
use crate::MAIN_SITE;
use crate::PROJECT;

/// The main site's people. Ada keeps the place.
#[table(name = "users")]
#[policy(all, |user, cx| cx.signed_in(|me| user.id.eq(me.account)))]
pub struct User {
	#[db(on_delete = cascade)]
	pub id: Id<Account>,
	pub admin: bool,
}

impl OnSignUp for User {
	async fn on_sign_up(tx: &Tx, account: Id<Account>, profile: &Profile) -> AuthResult<()> {
		let admin = profile
			.email
			.as_ref()
			.is_some_and(|email| email.to_string() == "ada@notes.example");
		NewUser { id: account, admin }.insert(tx).await?;
		Ok(())
	}
}

/// What the main site tells the admin about who may manage it.
impl Roles for User {
	async fn roles(db: &Db, account: Id<Account>) -> AuthResult<Vec<String>> {
		match User::find(db, account).await? {
			Some(user) if user.admin => Ok(vec!["admin".to_owned()]),
			_ => Ok(Vec::new()),
		}
	}
}

pub struct MainSite {
	pub router: Router,
	pub client_id: String,
	pub client_secret: String,
}

/// Sets up the main site, registers the admin as its first-party client,
/// and reports the site's request metrics to the control store.
pub async fn start(store: Store) -> Result<MainSite> {
	let db = Db::connect("sqlite::memory:").await?;
	leptoskit_db::testing::create_tables(&db).await?;

	let auth = Auth::builder(&db, Keys::single("main-site-demo", [5; 32]))
		.app_name("Notes")
		.origin(MAIN_SITE)
		.mailer(Mailbox::new())
		.dev_sign_in()
		.on_sign_up::<User>()
		.roles::<User>()
		.build()?;
	let idp = Idp::builder(&db, &auth)
		.sign_in_page("/sign-in")
		.consent_page("/consent")
		.build()?;

	let ClientCommandOutput::Added { client, secret } = idp
		.run_command(&ClientCommand::Add(NewClientSpec {
			name: "Notes admin".into(),
			redirect_uris: vec![format!("{ADMIN}/auth/callback/main")],
			post_logout_redirect_uris: vec![format!("{ADMIN}{}/sign-in", leptoskit_admin::BASE)],
			scopes: ["openid", "email", "profile", "roles"]
				.map(str::to_owned)
				.to_vec(),
			auth_method: ClientAuth::SecretPost,
			grant_types: leptoskit_idp::default_grant_types(),
			jwks: None,
			id_token_alg: SigningAlgorithm::Es256,
			first_party: true,
			introspection: false,
			dpop_bound: false,
		}))
		.await
	else {
		anyhow::bail!("the main site refused to register the admin");
	};
	let client_secret = secret.ok_or_else(|| anyhow::anyhow!("no client secret issued"))?;

	let router = Router::new()
		.route("/", get(home))
		.route("/sign-in", get(sign_in))
		.merge(auth.router())
		.merge(idp.router());
	// The IdP answers discovery and tokens from its layer, so the layers go
	// on last, outside the metrics.
	let (router, analytics) = leptoskit_apm::server::mount(
		router,
		leptoskit_apm::server::ServerConfig {
			export: None,
			resource: leptoskit_apm::Resource {
				service: PROJECT.into(),
				environment: "production".into(),
				release: "demo-1".into(),
				instance: "web-0".into(),
			},
			limits: leptoskit_apm::Limits::default(),
			read_key: "admin-demo-analytics-read-key-0123456789".into(),
			public_origin: format!("{MAIN_SITE}/"),
			browser_routes: Vec::new(),
		},
	)?;
	let router = router.layer(auth.layer()).layer(idp.layer());

	// What the main site served reaches the admin's health tab, as a real
	// deployment's metrics would.
	tokio::spawn(async move {
		let mut every = tokio::time::interval(Duration::from_secs(10));

		loop {
			every.tick().await;

			if let Err(error) = store.ingest(PROJECT, &analytics.snapshot()) {
				eprintln!("couldn't record the main site's metrics: {error}");
			}
		}
	});

	Ok(MainSite {
		router,
		client_id: client.client_id,
		client_secret,
	})
}

const STYLE: &str = "<meta name=viewport content='width=device-width'><style>body{font:16px \
                     system-ui;max-width:28rem;margin:4rem auto;padding:0 \
                     1rem;color:#222}button{display:block;width:100%;margin:.5rem \
                     0;padding:.7rem;font:inherit;border:1px solid \
                     #999;border-radius:8px;background:#fff;cursor:pointer}small{color:#666}</\
                     style>";

async fn home() -> Html<String> {
	Html(format!(
		"<!doctype html><meta charset=utf-8><title>Notes</title>{STYLE}<h1>Notes</h1><p>The main \
		 site. Its admin is at <a href='{ADMIN}{base}'>{ADMIN}{base}</a>.</p>",
		base = leptoskit_admin::BASE
	))
}

#[derive(serde::Deserialize)]
struct SignIn {
	return_to: Option<String>,
}

/// The main site's sign-in page, cut down to two buttons: the demo signs
/// people in without a code.
async fn sign_in(Query(query): Query<SignIn>) -> Html<String> {
	let return_to = query
		.return_to
		.filter(|path| path.starts_with('/') && !path.starts_with("//"))
		.unwrap_or_else(|| "/".to_owned());
	// A JSON string, with `<` escaped so the path can't close the script.
	let target = serde_json::to_string(&return_to)
		.unwrap_or_else(|_| "\"/\"".to_owned())
		.replace('<', "\\u003c");

	Html(format!(
		"<!doctype html><meta charset=utf-8><title>Sign in · Notes</title>{STYLE}<h1>Sign in to \
		 Notes</h1><button data-email=ada@notes.example>Ada <small>admin</small></button><button \
		 data-email=grace@notes.example>Grace <small>not an admin</small></button><script \
		 type=module>for(const b of \
		 document.querySelectorAll('button'))b.onclick=async()=>{{const r=await \
		 fetch('/auth/dev/sign-in',{{method:'POST',headers:{{'content-type':'application/json'}},\
		 body:JSON.stringify({{email:b.dataset.email}})}});if(r.ok)location.href={target};else \
		 b.textContent='Sign-in failed ('+r.status+')'}}</script>"
	))
}
