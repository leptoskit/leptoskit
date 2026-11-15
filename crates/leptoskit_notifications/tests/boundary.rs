//! The real HTTP boundary against local servers: redirects refused,
//! response reads bounded, pinned routing pinned, and HTTP/2 spoken.
//! A mock boundary proves the transports' logic; only a socket proves
//! the client's own behavior. No provider is contacted — every server
//! here is local, and the requests are the boundary's own shape.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use http_body_util::BodyExt as _;
use http_body_util::Full;
use http_body_util::combinators::BoxBody;
use hyper::Request;
use hyper::Response;
use hyper::body::Bytes;
use hyper::body::Incoming;
use hyper::service::service_fn;
use leptoskit_notifications::service::transport::Boundary as _;
use leptoskit_notifications::service::transport::HttpBoundary;
use leptoskit_notifications::service::transport::Pinned;
use leptoskit_notifications::service::transport::WireError;
use leptoskit_notifications::service::transport::WireRequest;

/// A local HTTP/1.1 server answering one handler, and its address.
async fn local_server(
	handler: impl Fn(Request<Incoming>) -> Response<BoxBody<Bytes, std::convert::Infallible>>
	+ Clone
	+ Send
	+ 'static,
) -> std::net::SocketAddr {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();

	tokio::spawn(async move {
		loop {
			let Ok((stream, _)) = listener.accept().await else {
				return;
			};
			let handler = handler.clone();
			let service = service_fn(move |request| {
				let handler = handler.clone();
				async move { Ok::<_, std::convert::Infallible>(handler(request)) }
			});

			tokio::spawn(async move {
				let _ = hyper::server::conn::http1::Builder::new()
					.serve_connection(tokio_io(stream), service)
					.await;
			});
		}
	});

	address
}

/// Tokio's compat IO for hyper.
fn tokio_io(stream: tokio::net::TcpStream) -> hyper_util::rt::TokioIo<tokio::net::TcpStream> {
	hyper_util::rt::TokioIo::new(stream)
}

/// A request for the boundary's own shape.
fn request(url: String) -> WireRequest {
	WireRequest {
		url,
		headers: Vec::new(),
		body: Vec::new(),
		pinned: None,
	}
}

/// A provider's 3xx is surfaced, never followed: the boundary's client
/// answers with the redirect status itself and sends exactly one
/// request, so nothing — a header, a cookie, a signed body — walks to
/// the redirect's target.
#[tokio::test(flavor = "multi_thread")]
async fn redirects_are_surfaced_never_followed() {
	let hits = Arc::new(AtomicUsize::new(0));
	let counted = Arc::clone(&hits);
	let address = local_server(move |request| {
		if request.uri().path() == "/landed" {
			counted.fetch_add(1, Ordering::SeqCst);
		}

		Response::builder()
			.status(302)
			.header("location", "/landed")
			.body(Full::new(Bytes::new()).boxed())
			.unwrap()
	})
	.await;

	let boundary = HttpBoundary::new().expect("the boundary builds");
	let answer = boundary
		.post(request(format!("http://{address}/redirect")))
		.await
		.expect("the redirect itself answers");

	assert_eq!(answer.status, 302, "the redirect is the answer");
	assert_eq!(hits.load(Ordering::SeqCst), 0, "the target was never asked");
}

/// A response body past the read bound is refused, not buffered: an
/// endless answer can't grow the service's memory waiting for its end.
#[tokio::test(flavor = "multi_thread")]
async fn an_endless_body_is_refused_at_the_bound() {
	let address = local_server(move |_| {
		let body = http_body_util::StreamBody::new(futures::stream::unfold(0_u32, |count| {
			async move {
				tokio::time::sleep(Duration::from_millis(2)).await;
				Some((
					Ok::<_, std::convert::Infallible>(hyper::body::Frame::data(Bytes::from(
						vec![b'x'; 4_096],
					))),
					count + 1,
				))
			}
		}));

		Response::builder().status(200).body(body.boxed()).unwrap()
	})
	.await;

	let boundary = HttpBoundary::new().expect("the boundary builds");
	let answer = boundary
		.post(request(format!("http://{address}/endless")))
		.await;

	assert!(
		matches!(answer, Err(WireError::TooLarge | WireError::TimedOut)),
		"the endless body is refused: {answer:?}"
	);
}

/// A pinned request connects only to the addresses its pin names: the
/// host in the URL is never resolved, and an address nothing listens on
/// is unreachable, not silently rerouted.
#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_request_connects_only_to_its_addresses() {
	let address = local_server(move |_| {
		Response::builder()
			.status(200)
			.body(Full::new(Bytes::from_static(b"pinned")).boxed())
			.unwrap()
	})
	.await;

	let boundary = HttpBoundary::new().expect("the boundary builds");
	let pinned = Pinned {
		host: "pinned.example.test".to_owned(),
		port: address.port(),
		addrs: vec!["127.0.0.1".parse().unwrap()],
	};
	let mut wire = request(format!(
		"http://pinned.example.test:{}/push",
		address.port()
	));
	wire.pinned = Some(pinned);

	let answer = boundary
		.post(wire)
		.await
		.expect("the pin reaches the server");
	assert_eq!(answer.status, 200);
	assert_eq!(answer.body, b"pinned");

	// A pin to an address nothing answers is unreachable — and never
	// falls back to resolving the host itself.
	let mut wire = request(format!(
		"http://pinned.example.test:{}/push",
		address.port()
	));
	wire.pinned = Some(Pinned {
		host: "pinned.example.test".to_owned(),
		port: address.port(),
		addrs: vec!["127.0.0.2".parse().unwrap()],
	});

	let answer = boundary.post(wire).await;
	assert!(
		matches!(answer, Err(WireError::Unreachable | WireError::TimedOut)),
		"the dead pin is refused: {answer:?}"
	);
}

/// The boundary's client speaks HTTP/2: APNs requires it, and the
/// production settings negotiate it over TLS. Over cleartext the same
/// builder settings need prior knowledge to choose h2c, so this runs
/// the production builder plus that one switch against a local server
/// that records the protocol it served — the wire behavior, with no
/// provider to touch.
#[tokio::test(flavor = "multi_thread")]
async fn the_boundary_client_speaks_http2() {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let served = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
	let recorded = Arc::clone(&served);

	// The auto builder sniffs the h2 preface, so one server answers
	// both HTTP/1.1 and prior-knowledge HTTP/2.
	tokio::spawn(async move {
		while let Ok((stream, _)) = listener.accept().await {
			tokio::spawn(serve_h2c(stream, Arc::clone(&recorded)));
		}
	});

	// The production client's own builder settings, with cleartext
	// prior knowledge standing in for what ALPN negotiates on TLS.
	let client = reqwest::Client::builder()
		.redirect(reqwest::redirect::Policy::none())
		.no_proxy()
		.connect_timeout(Duration::from_secs(5))
		.timeout(Duration::from_secs(10))
		.http2_prior_knowledge()
		.build()
		.expect("the client builds");
	let boundary = HttpBoundary::over_client(client);

	let answer = boundary
		.post(request(format!("http://{address}/push")))
		.await
		.expect("h2c answers");

	assert_eq!(answer.status, 200);
	assert_eq!(answer.body, b"h2");
	assert_eq!(
		served.lock().unwrap().as_slice(),
		["HTTP/2.0"],
		"the connection the server served was HTTP/2"
	);
}

/// Serves one h2c-or-http1 connection, recording the protocol it spoke.
async fn serve_h2c(stream: tokio::net::TcpStream, recorded: Arc<std::sync::Mutex<Vec<String>>>) {
	let service = service_fn(move |request: Request<Incoming>| {
		let recorded = Arc::clone(&recorded);

		async move {
			let version = format!("{:?}", request.version());
			recorded.lock().unwrap().push(version);

			Ok::<_, std::convert::Infallible>(Response::new(
				Full::new(Bytes::from_static(b"h2")).boxed(),
			))
		}
	});

	let _ = hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
		.serve_connection_with_upgrades(tokio_io(stream), service)
		.await;
}
