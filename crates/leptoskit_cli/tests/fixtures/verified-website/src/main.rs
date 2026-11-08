//! Small real server: RPC export precedes socket startup.
use std::io::{Read as _, Write as _};

fn main() {
    if let Some(path) = std::env::var_os("LEPTOSKIT_RPC_SPEC_OUT") {
        std::fs::write(path, r#"{"leptoskit_rpc_spec":3,"title":"website fixture","version":"0.1.0","functions":[]}"#).unwrap();

        return;
    }

    let listener = std::net::TcpListener::bind("0.0.0.0:3000").unwrap();

    for mut stream in listener.incoming().flatten() {
        let mut request = [0_u8; 1024];
        let n = stream.read(&mut request).unwrap_or(0);
        let text = String::from_utf8_lossy(&request[..n]);

        let requested = text.split_whitespace().nth(1).unwrap_or("/");

        let (path, content_type) = match requested {
            "/" => ("dist/index.html", "text/html; charset=utf-8"),
            "/pkg/website_app.js" => ("dist/pkg/website_app.js", "text/javascript"),
            "/pkg/website_app_bg.wasm" => ("dist/pkg/website_app_bg.wasm", "application/wasm"),
            "/assets/app.css" => ("dist/assets/app.css", "text/css"),
            _ => { let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"); continue; }
        };

        let body = std::fs::read(path).unwrap();
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n", body.len());
        let _ = stream.write_all(&body);
    }
}
