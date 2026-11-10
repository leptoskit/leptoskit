//! The conformance server's pages, driven as the suite's browser drives
//! them, without a browser: sign in with the button, allow with the
//! button, and trade the code without PKCE, as the Basic OP plan does.

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http_body_util::BodyExt as _;
use idp_conformance_example::SEEDED;
use idp_conformance_example::Settings;
use serde_json::Value;
use tower::ServiceExt as _;

const ISSUER: &str = "https://idp.test:9443";
const CALLBACK: &str = "https://suite.test:8443/test/a/leptoskit/callback";

async fn send(router: &Router, request: Request<Body>) -> Response {
	router.clone().oneshot(request).await.unwrap()
}

async fn text(response: Response) -> String {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	String::from_utf8(bytes.to_vec()).unwrap()
}

fn location(response: &Response) -> String {
	assert_eq!(response.status(), StatusCode::SEE_OTHER, "{response:?}");
	response.headers()[header::LOCATION]
		.to_str()
		.unwrap()
		.to_owned()
}

fn param(location: &str, name: &str) -> Option<String> {
	url::Url::parse(ISSUER)
		.unwrap()
		.join(location)
		.unwrap()
		.query_pairs()
		.find(|(key, _)| key == name)
		.map(|(_, value)| value.into_owned())
}

/// A form the browser posts from one of the server's own pages.
fn form(path: &str, pairs: &[(&str, &str)], cookie: Option<&str>) -> Request<Body> {
	let body = url::form_urlencoded::Serializer::new(String::new())
		.extend_pairs(pairs)
		.finish();
	let mut request = Request::post(path)
		.header(header::ORIGIN, ISSUER)
		.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
	if let Some(cookie) = cookie {
		request = request.header(header::COOKIE, cookie);
	}
	request.body(Body::from(body)).unwrap()
}

fn get(path: &str, cookie: Option<&str>) -> Request<Body> {
	let mut request = Request::get(path);
	if let Some(cookie) = cookie {
		request = request.header(header::COOKIE, cookie);
	}
	request.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn the_suite_signs_in_through_the_html_pages_without_pkce() {
	let server = idp_conformance_example::server(&Settings {
		issuer: ISSUER.into(),
		suite: "https://suite.test:8443/".into(),
		alias: "leptoskit".into(),
		rsa_key: include_str!("../../../crates/leptoskit_idp/tests/fixtures/test-only-rsa-key.pem")
			.into(),
	})
	.await
	.unwrap();
	let config = &server.suite_config;
	assert_eq!(
		config["server"]["discoveryUrl"],
		format!("{ISSUER}/.well-known/openid-configuration")
	);
	assert_eq!(
		config["browser"][0]["match"],
		format!("{ISSUER}/oauth/authorize*")
	);
	for name in ["client", "client2", "client_secret_post"] {
		assert!(config[name]["client_id"].is_string(), "{name}");
		assert!(config[name]["client_secret"].is_string(), "{name}");
	}
	let client_id = config["client"]["client_id"].as_str().unwrap();
	let secret = config["client"]["client_secret"].as_str().unwrap();
	let router = &server.router;

	// The suite asks without PKCE, and is sent to sign in.
	let query = url::form_urlencoded::Serializer::new(String::new())
		.extend_pairs([
			("response_type", "code"),
			("client_id", client_id),
			("redirect_uri", CALLBACK),
			("scope", "openid profile email"),
			("state", "s-1"),
			("nonce", "n-1"),
		])
		.finish();
	let to = location(&send(router, get(&format!("/oauth/authorize?{query}"), None)).await);
	assert!(to.starts_with("/sign-in?"), "{to}");
	let page = text(send(router, get(&to, None)).await).await;
	assert!(page.contains("id=sign-in"), "{page}");
	assert!(page.contains(SEEDED), "{page}");

	// The button signs the seeded person in and goes back.
	let return_to = param(&to, "return_to").unwrap();
	let signed_in = send(router, form("/sign-in", &[("return_to", &return_to)], None)).await;
	assert_eq!(location(&signed_in), return_to);
	let cookie = signed_in.headers()[header::SET_COOKIE]
		.to_str()
		.unwrap()
		.split(';')
		.next()
		.unwrap()
		.to_owned();
	let elsewhere = send(
		router,
		form("/sign-in", &[("return_to", "//evil.example")], None),
	)
	.await;
	assert_eq!(location(&elsewhere), "/");

	// Then consent, with the button.
	let to = location(&send(router, get(&return_to, Some(&cookie))).await);
	assert!(to.starts_with("/consent?"), "{to}");
	let page = text(send(router, get(&to, Some(&cookie))).await).await;
	assert!(page.contains("id=allow"), "{page}");
	assert!(page.contains("conformance client"), "{page}");
	let request = param(&to, "request").unwrap();
	let allowed = send(
		router,
		form(
			"/consent",
			&[("request", &request), ("approve", "yes")],
			Some(&cookie),
		),
	)
	.await;
	let to = location(&allowed);
	assert!(to.starts_with(CALLBACK), "{to}");
	assert_eq!(param(&to, "state").as_deref(), Some("s-1"));
	let code = param(&to, "code").unwrap();

	// The code trades without a verifier, for an RS256 ID token with the
	// nonce.
	let body = url::form_urlencoded::Serializer::new(String::new())
		.extend_pairs([
			("grant_type", "authorization_code"),
			("code", code.as_str()),
			("redirect_uri", CALLBACK),
		])
		.finish();
	let tokens = send(
		router,
		Request::post("/oauth/token")
			.header(
				header::AUTHORIZATION,
				format!("Basic {}", STANDARD.encode(format!("{client_id}:{secret}"))),
			)
			.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
			.body(Body::from(body))
			.unwrap(),
	)
	.await;
	assert_eq!(tokens.status(), StatusCode::OK);
	let tokens: Value = serde_json::from_str(&text(tokens).await).unwrap();
	let id_token = tokens["id_token"].as_str().unwrap();
	let part = |index: usize| -> Value {
		serde_json::from_slice(
			&URL_SAFE_NO_PAD
				.decode(id_token.split('.').nth(index).unwrap())
				.unwrap(),
		)
		.unwrap()
	};
	assert_eq!(part(0)["alg"], "RS256");
	assert_eq!(part(1)["nonce"], "n-1");

	// `userinfo` releases the seeded person's profile and address.
	let userinfo = send(
		router,
		Request::get("/oauth/userinfo")
			.header(
				header::AUTHORIZATION,
				format!("Bearer {}", tokens["access_token"].as_str().unwrap()),
			)
			.body(Body::empty())
			.unwrap(),
	)
	.await;
	let userinfo: Value = serde_json::from_str(&text(userinfo).await).unwrap();
	assert_eq!(userinfo["name"], "Conformance Tester");
	assert_eq!(userinfo["given_name"], "Conformance");
	assert_eq!(userinfo["email"], SEEDED);
	assert_eq!(userinfo["email_verified"], true);
}
