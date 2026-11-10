//! Rotating the RSA key, on SQLite and Postgres: a next key is published
//! before it signs, takes over once it has been published long enough, and
//! the key it replaced stays published only until the last token it signed
//! has expired. Time moves by moving the rows' timestamps, since the clock
//! is the database's own. The keys are made at test time.

#![cfg(all(feature = "axum", feature = "postgres", feature = "sqlite"))]

mod support;

use chrono::TimeDelta;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::ClientCommand;
use leptoskit_idp::ClientCommandOutput;
use leptoskit_idp::Idp;
use leptoskit_idp::NewClientSpec;
use leptoskit_idp::SigningAlgorithm;
use leptoskit_idp::SigningKey;
use leptoskit_idp::TokenResponse;
use postgres::PostgresCluster;
use serde_json::Value;
use support::*;

/// A fresh 2048-bit RSA key, as PEM.
fn rsa_key() -> String {
	let key = openssl::rsa::Rsa::generate(2048).unwrap();
	String::from_utf8(key.private_key_to_pem().unwrap()).unwrap()
}

/// The `kid` a token's header names.
fn kid(token: &str) -> String {
	let header: Value = serde_json::from_slice(
		&base64::Engine::decode(
			&base64::engine::general_purpose::URL_SAFE_NO_PAD,
			token.split('.').next().unwrap(),
		)
		.unwrap(),
	)
	.unwrap();
	header["kid"].as_str().unwrap().to_owned()
}

/// The RSA keys the key set publishes.
async fn published(app: &App) -> Vec<String> {
	let jwks: Value = body(app.get("/oauth/jwks", None).await).await;
	jwks["keys"]
		.as_array()
		.unwrap()
		.iter()
		.filter(|key| key["kty"] == "RSA")
		.map(|key| key["kid"].as_str().unwrap().to_owned())
		.collect()
}

/// The stored row of the key `kid`.
async fn row(app: &App, kid: &str) -> SigningKey {
	SigningKey::query()
		.filter(|key| key.kid.eq(kid.to_owned()))
		.first(&app.db.system("test: read a signing key"))
		.await
		.unwrap()
		.unwrap()
}

/// An RS256 ID token for whoever `cookie` signs in, from the app
/// `client_id`.
async fn id_token(app: &App, cookie: &str, client_id: &str, secret: &str) -> String {
	let code = app.code(client_id, "openid", cookie).await;
	let tokens: TokenResponse =
		body(app.exchange(client_id, Some(secret), &code, VERIFIER).await).await;
	tokens.id_token.unwrap()
}

#[tokio::test]
async fn a_next_rsa_key_is_published_first_then_takes_over_from_the_old_one() {
	let cluster = PostgresCluster::start();
	let (old, new, third) = (rsa_key(), rsa_key(), rsa_key());

	for db in databases(cluster.as_ref()).await {
		let before = {
			let old = old.clone();
			app_with(db.clone(), move |idp| idp.rsa_key(old)).await
		};
		let output = before
			.idp
			.run_command(&ClientCommand::Add(NewClientSpec {
				id_token_alg: SigningAlgorithm::Rs256,
				..spec(ClientAuth::SecretBasic, &["openid"])
			}))
			.await;
		let ClientCommandOutput::Added { client, secret } = output else {
			panic!("the client should register: {output:?}");
		};
		let (client_id, secret) = (client.client_id, secret.unwrap());
		// One sign-in: every app below is the same server over the same
		// database, so its cookie works for each.
		let cookie = before.sign_in("ada@example.com").await;
		let first = id_token(&before, &cookie, &client_id, &secret).await;
		let old_kid = kid(&first);
		assert_eq!(published(&before).await, std::slice::from_ref(&old_kid));

		// Deployed with a next key: it's published at once, but the old key
		// still signs.
		let app = {
			let (old, new) = (old.clone(), new.clone());
			restart(db.clone(), move |idp| idp.rsa_key(old).next_rsa_key(new)).await
		};
		let keys = published(&app).await;
		assert_eq!(keys.len(), 2, "{keys:?}");
		let new_kid = keys.iter().find(|kid| **kid != old_kid).unwrap().clone();
		let waiting = id_token(&app, &cookie, &client_id, &secret).await;
		assert_eq!(kid(&waiting), old_kid);
		assert!(row(&app, &old_kid).await.retires_at > leptoskit_db::now());

		// Almost a day later, still the old one.
		let published_at = leptoskit_db::now() - TimeDelta::hours(23);
		app.set_time(
			"signing_keys",
			"created_at",
			published_at,
			&format!("kid = '{new_kid}'"),
		)
		.await;
		assert_eq!(
			kid(&id_token(&app, &cookie, &client_id, &secret).await),
			old_kid
		);

		// A day after it was published, the next key signs, and the old one
		// stays published only as long as its last token lasts.
		let published_at = leptoskit_db::now() - TimeDelta::hours(25);
		app.set_time(
			"signing_keys",
			"created_at",
			published_at,
			&format!("kid = '{new_kid}'"),
		)
		.await;
		let switched = id_token(&app, &cookie, &client_id, &secret).await;
		assert_eq!(kid(&switched), new_kid);
		assert_eq!(verify_with_jwks(&app, &switched).await, "RS256");
		let retired = row(&app, &old_kid).await;
		let now = leptoskit_db::now();
		assert!(retired.retires_at <= now);
		assert!(retired.expires_at > now);
		assert!(retired.expires_at <= now + TimeDelta::minutes(10) + TimeDelta::hours(1));
		assert_eq!(verify_with_jwks(&app, &waiting).await, "RS256");
		assert_eq!(published(&app).await.len(), 2);

		// Signing more doesn't bring the old key back.
		id_token(&app, &cookie, &client_id, &secret).await;
		assert_eq!(row(&app, &old_kid).await.retires_at, retired.retires_at);

		// Once its last token has expired, the old key leaves the key set.
		app.set_time(
			"signing_keys",
			"expires_at",
			leptoskit_db::now() - TimeDelta::seconds(1),
			&format!("kid = '{old_kid}'"),
		)
		.await;
		assert_eq!(published(&app).await, std::slice::from_ref(&new_kid));

		// Making the next key the current one changes nothing more.
		let promoted = {
			let new = new.clone();
			restart(db.clone(), move |idp| idp.rsa_key(new)).await
		};
		let after = id_token(&promoted, &cookie, &client_id, &secret).await;
		assert_eq!(kid(&after), new_kid);
		assert_eq!(published(&promoted).await, std::slice::from_ref(&new_kid));

		// A key replaced outright, after a leak, stops signing at once and
		// leaves the key set with its last token, not a signing period later.
		let leaked = row(&promoted, &new_kid).await;
		assert!(leaked.expires_at > leptoskit_db::now() + TimeDelta::days(20));
		let replaced = {
			let third = third.clone();
			restart(db.clone(), move |idp| idp.rsa_key(third)).await
		};
		let fresh = id_token(&replaced, &cookie, &client_id, &secret).await;
		assert_ne!(kid(&fresh), new_kid);
		let cut = row(&replaced, &new_kid).await;
		let now = leptoskit_db::now();
		assert!(cut.retires_at <= now);
		assert!(cut.expires_at <= now + TimeDelta::minutes(10) + TimeDelta::hours(1));
		assert_eq!(published(&replaced).await.len(), 2);
	}
}

#[tokio::test]
async fn a_next_rsa_key_needs_a_current_one_and_must_differ() {
	let db = leptoskit_db::Db::connect("sqlite::memory:").await.unwrap();
	let key = rsa_key();
	let auth = leptoskit_auth::Auth::builder(&db, leptoskit_auth::Keys::single("test", [5; 32]))
		.origin(ORIGIN)
		.mailer(leptoskit_auth::Mailbox::new())
		.build()
		.unwrap();

	let alone = Idp::builder(&db, &auth).next_rsa_key(key.clone()).build();
	assert!(alone.unwrap_err().to_string().contains("set rsa_key too"));
	let same = Idp::builder(&db, &auth)
		.rsa_key(key.clone())
		.next_rsa_key(key)
		.build();
	assert!(same.unwrap_err().to_string().contains("is the current one"));
}
