//! The main site: accounts, sign-in, and the identity provider the admin
//! signs in through. Its own requests feed the health tab.

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use axum::Extension;
use axum::Json;
use axum::Router;
use axum::extract::Query;
use axum::response::Html;
use axum::routing::get;
use axum::routing::post;
use leptoskit_analytics::client::server::Analytics;
use leptoskit_analytics::client::server::Config as AnalyticsConfig;
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
use crate::analytics;

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
pub async fn start(store: Store, data: &Path) -> Result<MainSite> {
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

	// The analytics service, beside the site as `lk dev` runs it, and the
	// site's own server sending typed events to it.
	let service = analytics::start(data, MAIN_SITE).await?;
	let events = Analytics::new(
		MAIN_SITE,
		Some(analytics::TOKEN.to_owned()),
		AnalyticsConfig::default(),
	);

	let router = Router::new()
		.route("/", get(home))
		.route("/pricing", get(pricing))
		.route("/notes", get(notes))
		.route("/sign-in", get(sign_in))
		.route("/demo/sign-up", post(sign_up))
		.route("/demo/checkout", post(checkout))
		.route("/demo/note", post(note))
		.layer(Extension(events))
		.merge(service)
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
                     #999;border-radius:8px;background:#fff;cursor:pointer}small{color:#\
                     666}nav{margin-bottom:2rem}</style>";

/// What every page of the site runs: an anonymous id kept in the browser,
/// a session, and a `$pageview` sent to the analytics service's `capture`
/// operation, as the web client does. Buttons with `data-post` call the
/// site's own server, which sends the typed events.
const CAPTURE: &str = r#"<script type=module>
const uuid7 = () => {
  const b = crypto.getRandomValues(new Uint8Array(16));
  let t = Date.now();
  for (let i = 5; i >= 0; i--) { b[i] = t % 256; t = Math.floor(t / 256); }
  b[6] = (b[6] & 15) | 0x70; b[8] = (b[8] & 63) | 0x80;
  const h = [...b].map((x) => x.toString(16).padStart(2, "0")).join("");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
};
const anon = localStorage.getItem("notes-anon") ?? `anon-web-${uuid7().slice(-8)}`;
localStorage.setItem("notes-anon", anon);
const who = () => localStorage.getItem("notes-member") ?? anon;
const session = sessionStorage.getItem("notes-session") ?? uuid7();
sessionStorage.setItem("notes-session", session);
const mobile = /Mobi|Android|iPhone/.test(navigator.userAgent);
fetch("/_leptoskit/services/analytics/rpc/capture", {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ batch: [{
    event: "$pageview", distinct_id: who(), uuid: uuid7(), session_id: session,
    properties: {
      $current_url: location.href, $pathname: location.pathname, $referrer: document.referrer,
      $device_type: mobile ? "Mobile" : "Desktop",
    },
  }] }),
});
for (const button of document.querySelectorAll("[data-post]")) {
  button.onclick = async () => {
    const response = await fetch(button.dataset.post, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ who: who(), anon, session, plan: button.dataset.plan ?? "pro" }),
    });
    const answer = await response.json().catch(() => ({}));
    if (answer.member) localStorage.setItem("notes-member", answer.member);
    button.nextElementSibling.textContent = answer.message ?? `Failed (${response.status})`;
  };
}
</script>"#;

fn page(title: &str, body: &str) -> Html<String> {
	Html(format!(
		"<!doctype html><meta charset=utf-8><title>{title} · Notes</title>{STYLE}<nav><a \
		 href=/>Notes</a> · <a href=/pricing>Pricing</a> · <a href=/notes>Your \
		 notes</a></nav>{body}{CAPTURE}"
	))
}

async fn home() -> Html<String> {
	page(
		"Notes",
		&format!(
			"<h1>Notes</h1><p>The main site. Each page you open here is a <code>$pageview</code> \
			 in its analytics; its admin is at <a \
			 href='{ADMIN}{base}'>{ADMIN}{base}</a>.</p><button data-post=/demo/sign-up \
			 data-plan=free>Sign up</button><small></small>",
			base = leptoskit_admin::BASE
		),
	)
}

async fn pricing() -> Html<String> {
	page(
		"Pricing",
		"<h1>Pricing</h1><p>Each button sends a typed event from the site's server: \
		 <code>CheckoutStarted</code>, <code>PaymentSubmitted</code>, \
		 <code>CheckoutCompleted</code>.</p><button data-post=/demo/checkout?step=started \
		 data-plan=pro>Start checkout (Pro)</button><small></small><button \
		 data-post=/demo/checkout?step=paid data-plan=pro>Pay</button><small></small><button \
		 data-post=/demo/checkout?step=completed data-plan=pro>Finish</button><small></small>",
	)
}

async fn notes() -> Html<String> {
	page(
		"Your notes",
		"<h1>Your notes</h1><p>Writing one sends <code>NoteCreated</code>.</p><button \
		 data-post=/demo/note>Write a note</button><small></small>",
	)
}

#[derive(serde::Deserialize)]
struct Visitor {
	who: String,
	anon: String,
	#[serde(default)]
	plan: Option<String>,
}

#[derive(serde::Deserialize)]
struct Step {
	step: String,
}

/// Signs the visitor up: `SignedUp`, then identify, which joins their
/// anonymous visits to their new account.
async fn sign_up(
	Extension(events): Extension<Analytics>,
	Json(visitor): Json<Visitor>,
) -> Json<serde_json::Value> {
	let plan = visitor.plan.unwrap_or_else(|| "free".to_owned());
	let email = format!("{}@visitors.notes.example", visitor.anon);
	events.capture_event(
		&visitor.anon,
		analytics::SignedUp {
			plan: plan.clone(),
			source: "demo".to_owned(),
		},
	);
	events.identify(
		&email,
		&visitor.anon,
		vec![
			("email".into(), email.as_str().into()),
			("plan".into(), plan.into()),
		],
	);
	Json(serde_json::json!({
		"member": email,
		"message": format!("Signed up as {email}: SignedUp sent, and identify joined your visits."),
	}))
}

async fn checkout(
	Extension(events): Extension<Analytics>,
	Query(step): Query<Step>,
	Json(visitor): Json<Visitor>,
) -> Json<serde_json::Value> {
	let plan = visitor.plan.unwrap_or_else(|| "pro".to_owned());
	let sent = match step.step.as_str() {
		"started" => {
			events.capture_event(&visitor.who, analytics::CheckoutStarted { plan, seats: 1 });
			"CheckoutStarted"
		}
		"paid" => {
			events.capture_event(
				&visitor.who,
				analytics::PaymentSubmitted {
					plan,
					method: "card".to_owned(),
				},
			);
			"PaymentSubmitted"
		}
		_ => {
			events.capture_event(
				&visitor.who,
				analytics::CheckoutCompleted {
					plan,
					seats: 1,
					amount_cents: 800,
				},
			);
			"CheckoutCompleted"
		}
	};
	Json(serde_json::json!({ "message": format!("{sent} sent for {}.", visitor.who) }))
}

async fn note(
	Extension(events): Extension<Analytics>,
	Json(visitor): Json<Visitor>,
) -> Json<serde_json::Value> {
	events.capture_event(
		&visitor.who,
		analytics::NoteCreated {
			words: 120,
			shared: false,
		},
	);
	Json(serde_json::json!({ "message": format!("NoteCreated sent for {}.", visitor.who) }))
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
