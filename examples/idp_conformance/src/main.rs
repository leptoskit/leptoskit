//! Runs the conformance test server over https.
//!
//! Every setting comes from the environment:
//!
//! | Variable              | What it is                                                      |
//! | --------------------- | --------------------------------------------------------------- |
//! | `IDP_ISSUER`          | The issuer, such as `https://idp.test:9443`                     |
//! | `IDP_LISTEN`          | The address to listen on, such as `0.0.0.0:9443`                |
//! | `IDP_TLS_CERT`        | The PEM certificate chain file for the issuer's host            |
//! | `IDP_TLS_KEY`         | The PEM private key file for that certificate                   |
//! | `IDP_RSA_KEY`         | The RSA private key, as PEM, that signs RS256 ID tokens         |
//! | `CONFORMANCE_SUITE`   | The suite's address, such as `https://localhost.emobix.co.uk:8443` |
//! | `CONFORMANCE_ALIAS`   | The suite's alias for this server; `leptoskit` by default       |
//! | `CONFORMANCE_CONFIG`  | Where to write the suite's configuration                       |

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use idp_conformance_example::Settings;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use rustls::pki_types::pem::PemObject as _;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

/// How long a client may take to finish the TLS handshake.
const HANDSHAKE: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	let var = |name: &str| std::env::var(name).with_context(|| format!("set {name}"));
	let settings = Settings {
		issuer: var("IDP_ISSUER")?,
		suite: var("CONFORMANCE_SUITE")?,
		alias: std::env::var("CONFORMANCE_ALIAS").unwrap_or_else(|_| "leptoskit".into()),
		rsa_key: var("IDP_RSA_KEY")?,
	};
	let server = idp_conformance_example::server(&settings).await?;
	let config = var("CONFORMANCE_CONFIG")?;
	std::fs::write(&config, serde_json::to_vec_pretty(&server.suite_config)?)
		.with_context(|| format!("couldn't write {config}"))?;

	let acceptor = acceptor(&var("IDP_TLS_CERT")?, &var("IDP_TLS_KEY")?)?;
	let address: SocketAddr = var("IDP_LISTEN")?.parse().context("IDP_LISTEN")?;
	let tcp = TcpListener::bind(address)
		.await
		.with_context(|| format!("couldn't listen on {address}"))?;
	println!("{} is listening on {address}", settings.issuer);
	axum::serve(Tls::new(tcp, acceptor)?, server.router).await?;
	Ok(())
}

fn acceptor(cert: &str, key: &str) -> anyhow::Result<TlsAcceptor> {
	let chain = CertificateDer::pem_file_iter(cert)
		.with_context(|| format!("couldn't read {cert}"))?
		.collect::<Result<Vec<_>, _>>()
		.with_context(|| format!("{cert} isn't a PEM certificate chain"))?;
	let key = PrivateKeyDer::from_pem_file(key)
		.with_context(|| format!("{key} isn't a PEM private key"))?;
	let config = rustls::ServerConfig::builder_with_provider(Arc::new(
		rustls::crypto::ring::default_provider(),
	))
	.with_safe_default_protocol_versions()?
	.with_no_client_auth()
	.with_single_cert(chain, key)?;
	Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Accepts TLS connections, each handshake on its own task, so a client
/// that stalls in the handshake holds up no one else. One that fails it, or
/// takes longer than [`HANDSHAKE`], is dropped.
struct Tls {
	ready: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
	local: SocketAddr,
}

impl Tls {
	fn new(tcp: TcpListener, acceptor: TlsAcceptor) -> std::io::Result<Self> {
		let local = tcp.local_addr()?;
		let (sender, ready) = mpsc::channel(64);
		tokio::spawn(async move {
			loop {
				let Ok((stream, address)) = tcp.accept().await else {
					tokio::time::sleep(Duration::from_millis(50)).await;
					continue;
				};
				let (sender, acceptor) = (sender.clone(), acceptor.clone());
				tokio::spawn(async move {
					if let Ok(Ok(stream)) =
						tokio::time::timeout(HANDSHAKE, acceptor.accept(stream)).await
					{
						let _ = sender.send((stream, address)).await;
					}
				});
			}
		});
		Ok(Self { ready, local })
	}
}

impl axum::serve::Listener for Tls {
	type Addr = SocketAddr;
	type Io = TlsStream<TcpStream>;

	async fn accept(&mut self) -> (Self::Io, Self::Addr) {
		match self.ready.recv().await {
			Some(connection) => connection,
			// The accepting task never ends, so this never happens.
			None => std::future::pending().await,
		}
	}

	fn local_addr(&self) -> std::io::Result<Self::Addr> {
		Ok(self.local)
	}
}
