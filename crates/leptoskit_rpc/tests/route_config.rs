//! The routes `[rpc]` in `leptoskit.toml` configures:
//!
//! - the operation routes mount at `[rpc].path`, `/rpc` by default, with the
//!   legacy JSON routes kept on `/api` while JSON is a transport;
//! - the live socket mounts at `[rpc].live_path`, or `{path}/live`, instead of
//!   the default `/live`, with its server-sent events and frame posts right
//!   next to it;
//! - defaults keep today's `/rpc` and `/live`;
//! - a transport set without JSON refuses calls, and configuration loading
//!   rejects a transport that isn't JSON before a server is ever built.

#![cfg(all(feature = "live", feature = "sqlite"))]

use std::time::Duration;

use axum::Router;
use futures::StreamExt;
use http::Request;
use http::StatusCode;
use leptoskit_config::RpcConfig;
use leptoskit_rpc as rpc;
use leptoskit_rpc::live::ServerFrame;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite;
use tower::ServiceExt as _;

/// A query the tests reach over HTTP and the live socket.
#[rpc::query]
async fn hello(_cx: &rpc::QueryCx) -> rpc::Result<String> {
	Ok("hi".to_owned())
}

/// `POST path` against `app`, as JSON: the status and the body.
async fn post(app: &Router, path: &str) -> (StatusCode, String) {
	let response = app
		.clone()
		.oneshot(
			Request::post(path)
				.header("content-type", "application/json")
				.body(axum::body::Body::from("{}"))
				.unwrap(),
		)
		.await
		.unwrap();
	let status = response.status();
	let body = http_body_util::BodyExt::collect(response.into_body())
		.await
		.unwrap()
		.to_bytes();
	(status, String::from_utf8_lossy(&body).into_owned())
}

async fn serve(app: Router) -> std::net::SocketAddr {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
	address
}

/// The first server frame a fresh connection receives: its welcome.
async fn welcome(url: &str) -> ServerFrame {
	let (mut socket, _) = connect_async(url).await.unwrap();
	let message = tokio::time::timeout(Duration::from_secs(20), socket.next())
		.await
		.expect("a frame arrives")
		.unwrap()
		.unwrap();
	let tungstenite::Message::Text(text) = message else {
		panic!("expected a text frame: {message:?}");
	};
	serde_json::from_str(&text).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_configured_path_serves_operations_and_keeps_the_legacy_api() {
	let db = leptoskit_db::testing::sqlite().await;
	let config = RpcConfig {
		path: "/api/v2".into(),
		..RpcConfig::default()
	};
	let app = rpc::server::Server::new(db)
		.rpc_config(&config)
		.router::<()>();

	let (status, body) = post(&app, "/api/v2/hello").await;
	assert_eq!(status, StatusCode::OK, "{status}");
	assert_eq!(body, "\"hi\"");

	// The default route is gone.
	let (status, _) = post(&app, "/rpc/hello").await;
	assert_eq!(status, StatusCode::NOT_FOUND);

	// The legacy JSON prefix stays mounted while JSON is a transport.
	let (status, body) = post(&app, "/api/hello").await;
	assert_eq!(status, StatusCode::OK, "{status}");
	assert_eq!(body, "\"hi\"");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_defaults_keep_rpc_and_the_legacy_api() {
	let db = leptoskit_db::testing::sqlite().await;
	let app = rpc::server::Server::new(db).router::<()>();

	let (status, body) = post(&app, "/rpc/hello").await;
	assert_eq!(status, StatusCode::OK, "{status}");
	assert_eq!(body, "\"hi\"");

	let (status, body) = post(&app, "/api/hello").await;
	assert_eq!(status, StatusCode::OK, "{status}");
	assert_eq!(body, "\"hi\"");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transport_set_without_json_refuses_calls() {
	let db = leptoskit_db::testing::sqlite().await;
	// Built by hand: loading `leptoskit.toml` with this value fails earlier,
	// in the configuration's own validation.
	let config = RpcConfig {
		transports: vec!["pod".into()],
		..RpcConfig::default()
	};
	let app = rpc::server::Server::new(db)
		.rpc_config(&config)
		.router::<()>();

	let (status, body) = post(&app, "/rpc/hello").await;
	assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
	assert!(body.contains("JSON is not enabled"), "{body}");

	// Without JSON there are no JSON routes, so the legacy prefix is not
	// mounted either.
	let (status, _) = post(&app, "/api/hello").await;
	assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_configured_live_path_serves_the_socket() {
	let db = leptoskit_db::testing::sqlite().await;
	let config = RpcConfig {
		path: "/api/v2".into(),
		live_path: Some("/ws".into()),
		..RpcConfig::default()
	};
	let address = serve(
		rpc::server::Server::new(db)
			.rpc_config(&config)
			.live()
			.router(),
	)
	.await;

	let frame = welcome(&format!("ws://{address}/ws")).await;
	assert!(matches!(frame, ServerFrame::Welcome { .. }), "{frame:?}");

	// The events fallback sits next to the socket, and the default mount is
	// gone.
	let events = reqwest::get(format!("http://{address}/ws/events"))
		.await
		.unwrap();
	assert_ne!(events.status(), StatusCode::NOT_FOUND);
	let gone = reqwest::get(format!("http://{address}/live/events"))
		.await
		.unwrap();
	assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn live_follows_the_configured_path_without_an_explicit_live_path() {
	let db = leptoskit_db::testing::sqlite().await;
	let config = RpcConfig {
		path: "/api/v2".into(),
		..RpcConfig::default()
	};
	let address = serve(
		rpc::server::Server::new(db)
			.rpc_config(&config)
			.live()
			.router(),
	)
	.await;

	let frame = welcome(&format!("ws://{address}/api/v2/live")).await;
	assert!(matches!(frame, ServerFrame::Welcome { .. }), "{frame:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_defaults_keep_live_at_live() {
	let db = leptoskit_db::testing::sqlite().await;
	let config = RpcConfig::default();
	let address = serve(
		rpc::server::Server::new(db)
			.rpc_config(&config)
			.live()
			.router(),
	)
	.await;

	let frame = welcome(&format!("ws://{address}/live")).await;
	assert!(matches!(frame, ServerFrame::Welcome { .. }), "{frame:?}");
}
