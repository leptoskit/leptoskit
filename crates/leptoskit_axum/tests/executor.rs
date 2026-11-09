//! A served request can spawn through `any_spawner`, as leptos resources and
//! `leptoskit::query` do during server rendering. This file runs in its own
//! process, so nothing set the executor before the server started.

use std::time::Duration;

use axum::Router;
use axum::routing::get;
use leptoskit_axum::GracefulShutdown;
use leptoskit_axum::serve_listener;
use leptoskit_axum::serve_listener_with;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::oneshot;

/// Spawns a task the way a query cache does and answers with what it sent.
async fn spawned() -> String {
	let (send, receive) = oneshot::channel();
	any_spawner::Executor::spawn(async move {
		let _ = send.send("spawned".to_owned());
	});
	receive.await.expect("the spawned task ran")
}

async fn get_body(address: std::net::SocketAddr, path: &str) -> String {
	let mut stream = TcpStream::connect(address).await.expect("connect");
	let request = format!("GET {path} HTTP/1.1\r\nhost: test\r\nconnection: close\r\n\r\n");
	stream.write_all(request.as_bytes()).await.expect("write");
	let mut response = String::new();
	stream.read_to_string(&mut response).await.expect("read");
	response
}

#[tokio::test(flavor = "multi_thread")]
async fn serving_sets_tokio_as_the_executor_for_server_rendering() {
	let app = Router::new().route("/spawn", get(spawned));
	let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
	let address = listener.local_addr().expect("address");
	let (stop, stopped) = oneshot::channel::<()>();
	let shutdown = GracefulShutdown::new()
		.drain_delay(Duration::ZERO)
		.grace_period(Duration::from_secs(5))
		.signal(async move {
			let _ = stopped.await;
		});
	let server = tokio::spawn(serve_listener_with(listener, app, shutdown));

	let response = get_body(address, "/spawn").await;
	assert!(response.starts_with("HTTP/1.1 200"), "{response}");
	assert!(response.ends_with("spawned"), "{response}");

	// A second server in the same process keeps the executor it found.
	let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
	let second = listener.local_addr().expect("address");
	let again = tokio::spawn(serve_listener(
		listener,
		Router::new().route("/spawn", get(spawned)),
	));
	assert!(get_body(second, "/spawn").await.ends_with("spawned"));
	again.abort();

	stop.send(()).expect("signal");
	server.await.expect("join").expect("served");
}
