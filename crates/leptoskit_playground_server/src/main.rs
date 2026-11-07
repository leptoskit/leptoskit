//! Standalone host; the router can also be mounted in another service.
use leptoskit_playground_server::Config;
use leptoskit_playground_server::router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	let config = Config::from_env()?;
	let listener = tokio::net::TcpListener::bind(config.bind).await?;
	let app = router(config).await?;
	axum::serve(
		listener,
		app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
	)
	.with_graceful_shutdown(async {
		let _ = tokio::signal::ctrl_c().await;
	})
	.await?;
	Ok(())
}
