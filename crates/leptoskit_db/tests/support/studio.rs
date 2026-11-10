//! Talking to a running studio over HTTP, as a browser would. The tests
//! that use it include it by path, so the others carry none of it.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

/// A response: its status, headers, and body.
pub struct Reply {
	pub status: u16,
	pub head: String,
	pub body: String,
}

/// Sends one HTTP/1.1 request and reads the whole reply.
pub async fn send(address: SocketAddr, request: &str) -> Reply {
	let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
	stream.write_all(request.as_bytes()).await.unwrap();
	let mut reply = String::new();
	tokio::time::timeout(Duration::from_secs(10), stream.read_to_string(&mut reply))
		.await
		.expect("the studio answers")
		.unwrap();
	let (head, body) = reply.split_once("\r\n\r\n").unwrap();
	let status = head
		.split(' ')
		.nth(1)
		.and_then(|status| status.parse().ok())
		.unwrap();
	Reply {
		status,
		head: head.to_lowercase(),
		body: body.to_string(),
	}
}

fn encode(text: &str) -> String {
	text.bytes()
		.map(|byte| {
			if byte.is_ascii_alphanumeric() {
				char::from(byte).to_string()
			} else {
				format!("%{byte:02X}")
			}
		})
		.collect()
}

pub fn query(pairs: &[(&str, &str)]) -> String {
	pairs
		.iter()
		.map(|(name, value)| format!("{name}={}", encode(value)))
		.collect::<Vec<_>>()
		.join("&")
}

/// The address and token of the studio serving at `url`.
pub fn address_and_token(url: &str) -> (SocketAddr, String) {
	let (address, token) = url
		.strip_prefix("http://")
		.and_then(|rest| rest.split_once("/?token="))
		.unwrap();
	(address.parse().unwrap(), token.to_string())
}

/// `GET path` with `token` and `pairs`, to the studio at `address`.
pub async fn get(address: SocketAddr, token: &str, path: &str, pairs: &[(&str, &str)]) -> Reply {
	let mut all = vec![("token", token)];
	all.extend_from_slice(pairs);
	send(
		address,
		&format!(
			"GET {path}?{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
			query(&all),
			address.port()
		),
	)
	.await
}
