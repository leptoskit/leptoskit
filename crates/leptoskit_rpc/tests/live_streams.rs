//! Streams on a live server that has no database: `LiveStreams`, and the
//! `StreamCx` a body takes when it reads none.

#![cfg(feature = "live-streams")]

use std::pin::Pin;
use std::time::Duration;

use futures::SinkExt;
use futures::Stream;
use futures::StreamExt;
use leptoskit_rpc as rpc;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::server::Identity;
use leptoskit_rpc::server::LiveStreams;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// Counts to `upto` for whoever is calling, then ends.
#[rpc::stream]
pub async fn count_for(cx: &rpc::StreamCx, upto: u32) -> rpc::Result<impl Stream<Item = String>> {
	let caller = cx
		.identity()
		.caller
		.clone()
		.ok_or_else(|| rpc::Error::unauthorized("say who you are"))?;

	Ok(futures::stream::iter(
		(1..=upto).map(move |n| format!("{caller}:{n}")),
	))
}

/// Counts forever, slowly enough to be interrupted.
#[rpc::stream]
pub async fn count_forever(_cx: &rpc::StreamCx) -> rpc::Result<impl Stream<Item = u32>> {
	Ok(futures::stream::iter(0_u32..).chain(futures::stream::pending()))
}

/// Never opens: the doors are closed, and it says so by name.
#[rpc::stream]
pub async fn closed_doors(_cx: &rpc::StreamCx) -> rpc::Result<impl Stream<Item = u32>> {
	Err::<futures::stream::Empty<u32>, _>(rpc::Error::rate_limited(Duration::from_secs(30)))
}

/// A stream with no context at all.
#[rpc::stream]
pub async fn three() -> rpc::Result<impl Stream<Item = u32>> {
	Ok(futures::stream::iter(1_u32..=3))
}

/// Tells callers apart by an `x-caller` header or a bearer token.
struct Caller;

impl rpc::server::Identify for Caller {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> Pin<Box<dyn Future<Output = rpc::Result<Identity>> + Send + 'a>> {
		Box::pin(async move {
			let header = |name: &str| {
				request
					.headers
					.get(name)
					.and_then(|value| value.to_str().ok())
					.map(str::to_owned)
			};
			let caller = header("x-caller").or_else(|| {
				header("authorization")
					.and_then(|value| value.strip_prefix("Bearer ").map(str::to_owned))
			});

			if caller.as_deref() == Some("nobody") {
				return Err(rpc::Error::unauthorized("unknown token"));
			}

			Ok(caller.map_or_else(Identity::anonymous, |caller| {
				Identity::anonymous().with_caller(caller)
			}))
		})
	}
}

struct Server {
	live: LiveStreams,
	address: std::net::SocketAddr,
	task: JoinHandle<()>,
}

impl Drop for Server {
	fn drop(&mut self) {
		self.task.abort();
	}
}

async fn serve() -> Server {
	let live = LiveStreams::new().identify(Caller);
	let app = live.router::<()>();
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let task = tokio::spawn(async move {
		axum::serve(listener, app).await.unwrap();
	});

	Server {
		live,
		address,
		task,
	}
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(server: &Server, caller: Option<&str>) -> Socket {
	let mut request = format!("ws://{}/live", server.address)
		.into_client_request()
		.unwrap();

	if let Some(caller) = caller {
		request
			.headers_mut()
			.insert("x-caller", caller.parse().unwrap());
	}

	let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
	let ServerFrame::Welcome { version, .. } = next(&mut socket).await else {
		panic!("welcome first");
	};
	assert_eq!(version, 0, "there is no change log to have a version");
	socket
}

async fn send(socket: &mut Socket, frame: ClientFrame) {
	let text = serde_json::to_string(&frame).unwrap();
	socket.send(Message::Text(text.into())).await.unwrap();
}

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

async fn quiet(socket: &mut Socket) -> bool {
	tokio::time::timeout(Duration::from_millis(200), next(socket))
		.await
		.is_err()
}

async fn still_open(socket: &mut Socket) {
	send(socket, ClientFrame::Ping).await;
	assert_eq!(next(socket).await, ServerFrame::Pong);
}

fn open(stream: u32, name: &str, args: serde_json::Value, credits: u32) -> ClientFrame {
	ClientFrame::Stream {
		stream,
		name: name.into(),
		args,
		credits,
	}
}

async fn item(socket: &mut Socket, stream: u32) -> serde_json::Value {
	match next(socket).await {
		ServerFrame::Item {
			stream: from,
			value,
			..
		} if from == stream => value,
		other => panic!("expected an item of {stream}, got {other:?}"),
	}
}

async fn end(socket: &mut Socket, stream: u32) -> (String, Option<rpc::live::WireError>) {
	match next(socket).await {
		ServerFrame::End {
			stream: from,
			reason,
			err,
		} if from == stream => (reason, err),
		other => panic!("expected the end of {stream}, got {other:?}"),
	}
}

/// A stream runs as its caller, and only as fast as the client grants room.
#[tokio::test(flavor = "multi_thread")]
async fn a_stream_runs_as_the_caller_at_the_clients_pace() {
	let server = serve().await;
	let mut socket = connect(&server, Some("ada")).await;

	send(&mut socket, open(1, "count_for", json!({ "upto": 3 }), 2)).await;
	assert_eq!(item(&mut socket, 1).await, json!("ada:1"));
	assert_eq!(item(&mut socket, 1).await, json!("ada:2"));
	assert!(quiet(&mut socket).await, "the third item waits for room");

	send(&mut socket, ClientFrame::Credit { stream: 1, n: 5 }).await;
	assert_eq!(item(&mut socket, 1).await, json!("ada:3"));
	assert_eq!(end(&mut socket, 1).await, ("done".to_owned(), None));

	// A stream with no context argument opens too.
	send(&mut socket, open(2, "three", json!({}), 10)).await;
	for n in 1..=3 {
		assert_eq!(item(&mut socket, 2).await, json!(n));
	}
	assert_eq!(end(&mut socket, 2).await.0, "done");
	assert_eq!(server.live.connections(), 1);
}

/// What a body refuses with reaches the client whole: the status, and a
/// refusal's code and wait.
#[tokio::test(flavor = "multi_thread")]
async fn a_stream_that_will_not_open_says_why() {
	let server = serve().await;
	let mut socket = connect(&server, None).await;

	send(&mut socket, open(1, "count_for", json!({ "upto": 3 }), 2)).await;
	let (reason, err) = end(&mut socket, 1).await;
	let err = err.expect("why");
	assert_eq!((reason.as_str(), err.status), ("failed", 401));

	send(&mut socket, open(2, "closed_doors", json!({}), 2)).await;
	let (_, err) = end(&mut socket, 2).await;
	let err = err.expect("why");
	assert_eq!(err.status, 429);
	assert_eq!(err.code.as_deref(), Some(rpc::RATE_LIMITED));
	assert_eq!(err.retry_after, Some(30));

	send(&mut socket, open(3, "no_such_stream", json!({}), 2)).await;
	let (reason, err) = end(&mut socket, 3).await;
	assert_eq!(reason, "failed");
	assert!(err.expect("why").message.contains("no stream is named"));

	send(&mut socket, open(4, "count_for", json!({ "upto": "x" }), 2)).await;
	let (_, err) = end(&mut socket, 4).await;
	assert_eq!(err.expect("why").status, 400);
	still_open(&mut socket).await;
}

/// A streams-only server counts each stream a connection opens against
/// its caller, and a refused one ends without closing the connection.
#[tokio::test(flavor = "multi_thread")]
async fn opening_streams_is_rate_limited() {
	let live = LiveStreams::new()
		.identify(Caller)
		.client_ip_header("x-real-ip")
		.rate_limit(rpc::server::RateLimit::new().signed_in(rpc::server::Rate::per_minute(1)));
	let app = live.router::<()>();
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let task = tokio::spawn(async move {
		axum::serve(listener, app).await.unwrap();
	});
	let server = Server {
		live,
		address,
		task,
	};
	let mut socket = connect(&server, Some("ada")).await;

	send(&mut socket, open(1, "three", json!({}), 10)).await;
	for n in 1..=3 {
		assert_eq!(item(&mut socket, 1).await, json!(n));
	}
	assert_eq!(end(&mut socket, 1).await.0, "done");

	send(&mut socket, open(2, "three", json!({}), 10)).await;
	let (reason, err) = end(&mut socket, 2).await;
	let err = err.expect("why");
	assert_eq!((reason.as_str(), err.status), ("failed", 429));
	assert_eq!(err.code.as_deref(), Some(rpc::RATE_LIMITED));
	still_open(&mut socket).await;
}

/// Everything that needs a database is answered with why it isn't here,
/// and the connection stays open for streams.
#[tokio::test(flavor = "multi_thread")]
async fn what_needs_a_database_is_refused_by_name() {
	let server = serve().await;
	let mut socket = connect(&server, Some("ada")).await;
	let refused = |frame: ServerFrame, id: u32| {
		let ServerFrame::Done {
			request,
			err: Some(err),
			..
		} = frame
		else {
			panic!("expected a refusal, got {frame:?}");
		};
		assert_eq!((request, err.status), (id, 501));
		assert!(err.message.contains("streams only"), "{}", err.message);
	};

	send(
		&mut socket,
		ClientFrame::Sub {
			id: 7,
			query: "notes".into(),
			args: json!({}),
		},
	)
	.await;
	refused(next(&mut socket).await, 7);

	send(
		&mut socket,
		ClientFrame::Mutate {
			request: 8,
			name: "add_note".into(),
			args: json!({}),
			key: None,
		},
	)
	.await;
	refused(next(&mut socket).await, 8);

	send(
		&mut socket,
		ClientFrame::Act {
			request: 9,
			name: "send_mail".into(),
			args: json!({}),
		},
	)
	.await;
	refused(next(&mut socket).await, 9);

	send(
		&mut socket,
		ClientFrame::Join {
			channel: "lobby".into(),
			name: "room".into(),
			args: json!({}),
			state: json!(null),
		},
	)
	.await;
	let ServerFrame::ChannelError { channel, err } = next(&mut socket).await else {
		panic!("expected a channel error");
	};
	assert_eq!((channel.as_str(), err.status), ("lobby", 501));

	// Frames that ask for nothing are ignored, not refused.
	send(&mut socket, ClientFrame::Hello { resume: None }).await;
	send(&mut socket, ClientFrame::Unsub { id: 7 }).await;
	still_open(&mut socket).await;
}

/// Signing in over the socket stops the streams the connection opened as
/// someone else, and the next one runs as the new caller. A token nobody
/// holds closes the connection.
#[tokio::test(flavor = "multi_thread")]
async fn signing_in_interrupts_streams_and_changes_the_caller() {
	let server = serve().await;
	let mut socket = connect(&server, None).await;

	send(&mut socket, open(1, "count_forever", json!({}), 1)).await;
	assert_eq!(item(&mut socket, 1).await, json!(0));
	send(
		&mut socket,
		ClientFrame::Auth {
			token: "grace".into(),
		},
	)
	.await;
	assert_eq!(end(&mut socket, 1).await.0, "interrupted");
	assert_eq!(
		next(&mut socket).await,
		ServerFrame::AuthStatus {
			refused: false,
			expiring: false,
		}
	);

	send(&mut socket, open(2, "count_for", json!({ "upto": 1 }), 5)).await;
	assert_eq!(item(&mut socket, 2).await, json!("grace:1"));
	assert_eq!(end(&mut socket, 2).await.0, "done");

	send(
		&mut socket,
		ClientFrame::Auth {
			token: "nobody".into(),
		},
	)
	.await;
	assert_eq!(
		next(&mut socket).await,
		ServerFrame::AuthStatus {
			refused: true,
			expiring: false,
		}
	);
}

/// Cancelling stops a stream without an end, and draining tells the client
/// each open stream was interrupted and to come back.
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_and_draining_stop_streams() {
	let server = serve().await;
	let mut socket = connect(&server, Some("ada")).await;

	send(&mut socket, open(1, "count_forever", json!({}), 1)).await;
	assert_eq!(item(&mut socket, 1).await, json!(0));
	send(&mut socket, ClientFrame::Cancel { stream: 1 }).await;
	send(&mut socket, ClientFrame::Credit { stream: 1, n: 10 }).await;
	still_open(&mut socket).await;

	send(&mut socket, open(2, "count_forever", json!({}), 1)).await;
	assert_eq!(item(&mut socket, 2).await, json!(0));
	server.live.drain();
	assert_eq!(end(&mut socket, 2).await.0, "interrupted");
	assert!(matches!(
		next(&mut socket).await,
		ServerFrame::Reconnect { .. }
	));
}

/// The server-sent events fallback carries the same frames, with the
/// client's own posted to the connection `welcome` names.
#[tokio::test(flavor = "multi_thread")]
async fn streams_run_over_server_sent_events() {
	use tokio::io::AsyncReadExt;
	use tokio::io::AsyncWriteExt;

	let server = serve().await;
	let mut events = tokio::net::TcpStream::connect(server.address)
		.await
		.unwrap();
	events
		.write_all(
			format!(
				"GET /live/events HTTP/1.1\r\nhost: {}\r\nx-caller: ada\r\naccept: \
				 text/event-stream\r\n\r\n",
				server.address
			)
			.as_bytes(),
		)
		.await
		.unwrap();

	let mut seen = String::new();
	let mut read_until = async |events: &mut tokio::net::TcpStream, wanted: &str| {
		let mut buffer = [0_u8; 4096];

		while !seen.contains(wanted) {
			let read = tokio::time::timeout(Duration::from_secs(20), events.read(&mut buffer))
				.await
				.expect("the event stream keeps sending")
				.unwrap();
			assert!(read > 0, "the event stream closed: {seen}");
			seen.push_str(&String::from_utf8_lossy(&buffer[..read]));
		}

		seen.clone()
	};

	let text = read_until(&mut events, "\"connection\":\"").await;
	let connection = text
		.split("\"connection\":\"")
		.nth(1)
		.and_then(|rest| rest.split('"').next())
		.expect("the connection's name")
		.to_owned();
	let frames = serde_json::to_string(&[open(1, "count_for", json!({ "upto": 2 }), 5)]).unwrap();
	let mut post = tokio::net::TcpStream::connect(server.address)
		.await
		.unwrap();
	post.write_all(
		format!(
			"POST /live/frames?connection={connection} HTTP/1.1\r\nhost: {}\r\nx-caller: \
			 ada\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: \
			 close\r\n\r\n{frames}",
			server.address,
			frames.len()
		)
		.as_bytes(),
	)
	.await
	.unwrap();
	let mut answer = String::new();
	post.read_to_string(&mut answer).await.unwrap();
	assert!(answer.starts_with("HTTP/1.1 202"), "{answer}");

	let text = read_until(&mut events, "\"reason\":\"done\"").await;
	assert!(text.contains("ada:1"), "{text}");
	assert!(text.contains("ada:2"), "{text}");
}

/// With a database driver in the build, a stream that reads the database
/// says it can't on a server that has none, and the full live server opens
/// a stream that takes only the caller.
#[cfg(all(feature = "live", feature = "sqlite"))]
mod with_a_database {
	use super::*;

	/// Reads the database as the caller, so it needs one.
	#[rpc::stream]
	pub async fn needs_rows(_cx: &rpc::QueryCx) -> rpc::Result<impl Stream<Item = u32>> {
		Ok(futures::stream::iter(1_u32..=1))
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn a_stream_that_reads_the_database_needs_the_full_server() {
		let server = serve().await;
		let mut socket = connect(&server, Some("ada")).await;

		send(&mut socket, open(1, "needs_rows", json!({}), 5)).await;
		let (reason, err) = end(&mut socket, 1).await;
		let err = err.expect("why");
		assert_eq!((reason.as_str(), err.status), ("failed", 500));
		assert!(err.message.contains("streams only"), "{}", err.message);
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn the_full_server_opens_both_kinds() {
		let live = rpc::server::Live::new(leptoskit_db::testing::sqlite().await).identify(Caller);
		let app = live.router::<()>();
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
			.insert("x-caller", "ada".parse().unwrap());
		let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
		assert!(matches!(
			next(&mut socket).await,
			ServerFrame::Welcome { .. }
		));

		send(&mut socket, open(1, "count_for", json!({ "upto": 1 }), 5)).await;
		assert_eq!(item(&mut socket, 1).await, json!("ada:1"));
		assert_eq!(end(&mut socket, 1).await.0, "done");

		send(&mut socket, open(2, "needs_rows", json!({}), 5)).await;
		assert_eq!(item(&mut socket, 2).await, json!(1));
		assert_eq!(end(&mut socket, 2).await.0, "done");

		send(&mut socket, open(3, "three", json!({}), 5)).await;
		assert_eq!(item(&mut socket, 3).await, json!(1));
		task.abort();

		// `Live::streams_only()` is the same server as `LiveStreams::new()`.
		let _: LiveStreams = rpc::server::Live::streams_only();
	}
}
