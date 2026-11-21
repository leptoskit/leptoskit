//! What runs before every call a client sends: the server's rate limit,
//! each operation's own `rate_limit`, `timeout`, and `max_body`, and the
//! app's own guards, over HTTP and over a live connection.

#![cfg(all(feature = "live", feature = "sqlite"))]

use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::Request;
use axum::http::StatusCode;
use futures::SinkExt;
use futures::StreamExt;
use http_body_util::BodyExt;
use leptoskit_query::BoxFuture;
use leptoskit_rpc as rpc;
use leptoskit_rpc::Kind;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::server::Call;
use leptoskit_rpc::server::Guard;
use leptoskit_rpc::server::Identity;
use leptoskit_rpc::server::Rate;
use leptoskit_rpc::server::RateLimit;
use leptoskit_rpc::server::Server;
use leptoskit_rpc::server::Via;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;

/// Answers anyone, as often as the server lets them.
#[rpc::query]
pub async fn guarded_ping() -> rpc::Result<String> {
	Ok("pong".to_owned())
}

/// Answers each caller once a minute, whatever the server allows.
#[rpc::query(rate_limit = 1 / 1m)]
pub async fn guarded_once() -> rpc::Result<String> {
	Ok("once".to_owned())
}

/// Takes longer than it may.
#[rpc::query(timeout = 50ms)]
pub async fn guarded_slow() -> rpc::Result<()> {
	tokio::time::sleep(Duration::from_secs(10)).await;
	Ok(())
}

/// Takes at most a kibibyte of arguments.
#[rpc::mutation(max_body = 1KiB)]
pub async fn guarded_small(_cx: &rpc::MutationCx, text: String) -> rpc::Result<usize> {
	Ok(text.len())
}

/// Counts what it was sent, once a minute per caller, over a live
/// connection too.
#[rpc::mutation(rate_limit = 1 / 1m)]
pub async fn guarded_send(_cx: &rpc::MutationCx) -> rpc::Result<()> {
	Ok(())
}

/// Tells callers apart by an `x-caller` header.
struct Header;

impl rpc::server::Identify for Header {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> BoxFuture<'a, rpc::Result<Identity>> {
		let caller = request
			.headers
			.get("x-caller")
			.and_then(|value| value.to_str().ok())
			.map(str::to_owned);
		Box::pin(std::future::ready(Ok(caller
			.map_or_else(Identity::anonymous, |caller| {
				Identity::anonymous().with_caller(caller)
			}))))
	}
}

/// One call over HTTP.
async fn call(
	router: &Router,
	name: &str,
	body: &Value,
	from: [u8; 4],
	caller: Option<&str>,
) -> (StatusCode, Option<String>, String) {
	let mut request = Request::post(format!("/rpc/{name}"))
		.header("content-type", "application/json")
		.extension(ConnectInfo(SocketAddr::from((from, 4000))));

	if let Some(caller) = caller {
		request = request.header("x-caller", caller);
	}

	let response = router
		.clone()
		.oneshot(request.body(Body::from(body.to_string())).unwrap())
		.await
		.unwrap();
	let status = response.status();
	let retry_after = response
		.headers()
		.get("retry-after")
		.map(|value| value.to_str().unwrap().to_owned());
	let body = response.into_body().collect().await.unwrap().to_bytes();
	(
		status,
		retry_after,
		String::from_utf8(body.to_vec()).unwrap(),
	)
}

#[tokio::test]
async fn callers_are_limited_by_address_or_by_who_they_are() {
	let db = leptoskit_db::testing::sqlite().await;
	let router = Server::new(db)
		.identify(Header)
		.rate_limit(
			RateLimit::new()
				.anonymous(Rate::per_minute(2))
				.signed_in(Rate::per_minute(3)),
		)
		.router::<()>();
	let ping = json!({});

	for _ in 0..2 {
		let (status, _, body) = call(&router, "guarded_ping", &ping, [203, 0, 113, 1], None).await;
		assert_eq!((status, body.as_str()), (StatusCode::OK, "\"pong\""));
	}

	// The address has spent its allowance: `429`, with how long to wait,
	// in the shape every client reads as a refusal.
	let (status, retry_after, body) =
		call(&router, "guarded_ping", &ping, [203, 0, 113, 1], None).await;
	assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
	assert_eq!(retry_after.as_deref(), Some("30"));
	assert!(body.contains("\"code\":\"rate_limited\""), "{body}");
	let refused = rpc::FnError::from_wire(&body);
	assert_eq!(refused.code(), Some(rpc::RATE_LIMITED));

	// Another address has its own.
	let (status, ..) = call(&router, "guarded_ping", &ping, [203, 0, 113, 2], None).await;
	assert_eq!(status, StatusCode::OK);

	// Someone signed in, from the spent address, is counted as themselves.
	for _ in 0..3 {
		let (status, ..) = call(
			&router,
			"guarded_ping",
			&ping,
			[203, 0, 113, 1],
			Some("ada"),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
	}
	let (status, ..) = call(
		&router,
		"guarded_ping",
		&ping,
		[203, 0, 113, 1],
		Some("ada"),
	)
	.await;
	assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

	// A service the app calls is not counted.
	for _ in 0..5 {
		let (status, ..) = call(
			&router,
			"guarded_ping",
			&ping,
			[10, 0, 0, 1],
			Some("service:analytics"),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
	}
}

#[tokio::test]
async fn an_operation_keeps_its_own_limits_without_a_server_limit() {
	let db = leptoskit_db::testing::sqlite().await;
	let router = Server::new(db).identify(Header).max_body(64).router::<()>();
	let none = json!({});

	// `rate_limit = 1 / 1m`, per caller.
	let (status, ..) = call(&router, "guarded_once", &none, [198, 51, 100, 1], None).await;
	assert_eq!(status, StatusCode::OK);
	let (status, retry_after, _) =
		call(&router, "guarded_once", &none, [198, 51, 100, 1], None).await;
	assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
	assert_eq!(retry_after.as_deref(), Some("60"));
	let (status, ..) = call(&router, "guarded_once", &none, [198, 51, 100, 2], None).await;
	assert_eq!(status, StatusCode::OK);
	// No server-wide limit: other operations aren't counted.
	for _ in 0..10 {
		let (status, ..) = call(&router, "guarded_ping", &none, [198, 51, 100, 1], None).await;
		assert_eq!(status, StatusCode::OK);
	}

	// `timeout = 50ms`.
	let (status, _, body) = call(&router, "guarded_slow", &none, [198, 51, 100, 1], None).await;
	assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
	assert_eq!(
		rpc::FnError::from_wire(&body).code(),
		Some(rpc::server::TIMED_OUT)
	);

	// `max_body = 1KiB`, in place of the server's 64 bytes.
	let fits = json!({ "text": "a".repeat(900) });
	let (status, _, body) = call(&router, "guarded_small", &fits, [198, 51, 100, 1], None).await;
	assert_eq!((status, body.as_str()), (StatusCode::OK, "900"));
	let over = json!({ "text": "a".repeat(1100) });
	let (status, _, body) = call(&router, "guarded_small", &over, [198, 51, 100, 1], None).await;
	assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
	assert_eq!(
		rpc::FnError::from_wire(&body).code(),
		Some(rpc::server::TOO_LARGE)
	);

	// The server's `max_body` holds where the operation says nothing.
	let long = json!({ "padding": "a".repeat(100) });
	let (status, ..) = call(&router, "guarded_ping", &long, [198, 51, 100, 1], None).await;
	assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

/// What a guard was shown.
type Seen = Arc<Mutex<Vec<(String, Kind, Via, Option<IpAddr>)>>>;

/// Lets only people who are signed in call `guarded_once`, and writes down
/// every call it sees.
struct SignedInOnly(Seen);

impl Guard for SignedInOnly {
	fn check<'a>(&'a self, call: &'a Call<'a>) -> BoxFuture<'a, rpc::Result<()>> {
		self.0
			.lock()
			.unwrap()
			.push((call.name.to_owned(), call.kind, call.via, call.address));
		let allowed = call.name != "guarded_once" || call.identity.caller.is_some();
		Box::pin(std::future::ready(if allowed {
			Ok(())
		} else {
			Err(rpc::Error::unauthorized("sign in first"))
		}))
	}
}

#[tokio::test]
async fn a_guard_sees_each_call_and_may_refuse_it() {
	let db = leptoskit_db::testing::sqlite().await;
	let seen = Seen::default();
	let router = Server::new(db)
		.identify(Header)
		.client_ip_header("x-real-ip")
		.guard(SignedInOnly(Arc::clone(&seen)))
		.router::<()>();

	let (status, ..) = call(&router, "guarded_ping", &json!({}), [10, 0, 0, 1], None).await;
	assert_eq!(status, StatusCode::OK);
	let (status, _, body) = call(&router, "guarded_once", &json!({}), [10, 0, 0, 1], None).await;
	assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

	// Behind a proxy, the address is the one it writes.
	let response = router
		.clone()
		.oneshot(
			Request::post("/rpc/guarded_ping")
				.header("content-type", "application/json")
				.header("x-real-ip", "203.0.113.50")
				.extension(ConnectInfo(SocketAddr::from(([10, 0, 0, 1], 4000))))
				.body(Body::from("{}"))
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(response.status(), StatusCode::OK);

	assert_eq!(
		*seen.lock().unwrap(),
		[
			(
				"guarded_ping".to_owned(),
				Kind::Query,
				Via::Http,
				Some(IpAddr::from([10, 0, 0, 1]))
			),
			(
				"guarded_once".to_owned(),
				Kind::Query,
				Via::Http,
				Some(IpAddr::from([10, 0, 0, 1]))
			),
			(
				"guarded_ping".to_owned(),
				Kind::Query,
				Via::Http,
				Some(IpAddr::from([203, 0, 113, 50]))
			),
		]
	);
}

type Socket =
	tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next(socket: &mut Socket) -> ServerFrame {
	loop {
		let message = tokio::time::timeout(Duration::from_secs(20), socket.next())
			.await
			.expect("a frame arrives")
			.unwrap()
			.unwrap();

		match message {
			Message::Text(text) => return serde_json::from_str(&text).unwrap(),
			Message::Ping(_) | Message::Pong(_) => {}
			other => panic!("unexpected frame: {other:?}"),
		}
	}
}

async fn send(socket: &mut Socket, frame: ClientFrame) {
	let text = serde_json::to_string(&frame).unwrap();
	socket.send(Message::Text(text.into())).await.unwrap();
}

/// The answer to the frame `id`, or why it was refused.
async fn answer(socket: &mut Socket, id: u32) -> Result<Value, rpc::live::WireError> {
	loop {
		match next(socket).await {
			ServerFrame::Done {
				request, ok, err, ..
			} if request == id => {
				return err.map_or_else(|| Ok(ok.unwrap_or(Value::Null)), Err);
			}
			ServerFrame::Transition { .. } => {}
			other => panic!("expected the answer to {id}, got {other:?}"),
		}
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_connection_is_counted_frame_by_frame() {
	let db = leptoskit_db::testing::sqlite().await;
	let seen = Seen::default();
	let server = Server::new(db)
		.identify(Header)
		.client_ip_header("x-real-ip")
		.rate_limit(RateLimit::new().anonymous(Rate::per_minute(3)))
		.guard(SignedInOnly(Arc::clone(&seen)));
	let live = server.live();
	let app = live
		.router::<()>()
		.into_make_service_with_connect_info::<SocketAddr>();
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let task = tokio::spawn(async move {
		axum::serve(listener, app).await.unwrap();
	});

	let mut request = format!("ws://{address}/live")
		.into_client_request()
		.unwrap();
	request
		.headers_mut()
		.insert("x-real-ip", "198.51.100.7".parse().unwrap());
	let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
	let ServerFrame::Welcome { .. } = next(&mut socket).await else {
		panic!("welcome first");
	};

	// The operation's own limit: one a minute.
	send(
		&mut socket,
		ClientFrame::Mutate {
			request: 1,
			name: "guarded_send".into(),
			args: json!({}),
			key: None,
		},
	)
	.await;
	assert_eq!(answer(&mut socket, 1).await, Ok(Value::Null));
	send(
		&mut socket,
		ClientFrame::Mutate {
			request: 2,
			name: "guarded_send".into(),
			args: json!({}),
			key: None,
		},
	)
	.await;
	let refused = answer(&mut socket, 2).await.unwrap_err();
	assert_eq!(refused.status, 429);
	assert_eq!(refused.code.as_deref(), Some("rate_limited"));

	// The address's allowance of three is spent by the third call, and the
	// connection stays open.
	send(
		&mut socket,
		ClientFrame::Sub {
			id: 3,
			query: "guarded_ping".into(),
			args: json!({}),
		},
	)
	.await;
	assert_eq!(answer(&mut socket, 3).await, Ok(json!("pong")));
	send(
		&mut socket,
		ClientFrame::Sub {
			id: 4,
			query: "guarded_ping".into(),
			args: json!({}),
		},
	)
	.await;
	assert_eq!(answer(&mut socket, 4).await.unwrap_err().status, 429);
	send(&mut socket, ClientFrame::Ping).await;
	assert_eq!(next(&mut socket).await, ServerFrame::Pong);

	// The guard saw each frame that the rate limit let through, as live
	// calls from the proxy's address.
	let address = Some(IpAddr::from([198, 51, 100, 7]));
	assert_eq!(
		*seen.lock().unwrap(),
		[
			(
				"guarded_send".to_owned(),
				Kind::Mutation,
				Via::Live,
				address
			),
			("guarded_ping".to_owned(), Kind::Query, Via::Live, address),
		]
	);

	task.abort();
}
