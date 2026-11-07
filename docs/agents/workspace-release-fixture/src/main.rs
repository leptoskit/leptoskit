use std::io::Read;
use std::io::Write;
use std::net::TcpListener;

fn main() {
	if let Some(path) = std::env::var_os("LEPTOSKIT_RPC_SPEC_OUT") {
		std::fs::write(
			path,
			r#"{"leptoskit_rpc_spec":3,"title":"fixture","version":"0.1.0","functions":[]}"#,
		)
		.unwrap();
		return;
	}
	let address = std::env::var("LEPTOS_SITE_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".into());
	for stream in TcpListener::bind(address).unwrap().incoming() {
		let mut stream = stream.unwrap();
		stream
			.set_read_timeout(Some(std::time::Duration::from_secs(2)))
			.unwrap();
		let mut request = [0; 4096];
		let Ok(length) = stream.read(&mut request) else {
			continue;
		};
		let ready = request[..length].starts_with(b"GET /readyz HTTP/1.");
		let status = if ready { "200 OK" } else { "404 Not Found" };
		let body = if ready { "ready\n" } else { "missing\n" };
		let _ = write!(
			stream,
			"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
			body.len()
		);
	}
}
