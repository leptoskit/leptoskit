//! The globals a build with no live connection has. Its own test binary,
//! since the process's live client, once installed, stays.

#![cfg(all(feature = "live", feature = "sqlite"))]

use leptos::prelude::Get as _;
use leptos::prelude::Owner;
use leptoskit_rpc as rpc;
use leptoskit_rpc::client::ClientConfig;

/// A build with no installed client reports no connection, and the globals
/// do nothing.
#[test]
fn without_an_installed_client_the_globals_are_offline() {
	// Reading a signal needs an owner: reactive_graph is built with
	// sandboxed arenas, so an active one must hold the status signal.
	let owner = Owner::new();
	owner.set();

	rpc::client::configure(ClientConfig::new());
	assert!(!rpc::client::live::available());
	rpc::client::live::reauthenticate();
	// Signal hooks run inside a component owner, including builds using
	// sandboxed arenas.
	Owner::new().with(|| {
		let status = rpc::client::live::use_live_status();
		assert_eq!(status.get(), rpc::client::LiveStatus::OFFLINE);
	});
}

/// A note written through the ordinary client, without a live connection.
#[leptoskit_db::table(name = "offline_notes")]
pub struct OfflineNote {
	pub id: leptoskit_db::Id<Self>,
	pub body: String,
}

#[rpc::query]
/// Reads the notes through the ordinary in-process client.
pub async fn offline_bodies(cx: &rpc::QueryCx) -> rpc::Result<Vec<String>> {
	Ok(OfflineNote::query()
		.all(cx.db())
		.await?
		.into_iter()
		.map(|note| note.body)
		.collect())
}

#[rpc::mutation]
/// Writes a note through the ordinary in-process client.
pub async fn offline_add(cx: &rpc::MutationCx, body: String) -> rpc::Result<()> {
	NewOfflineNote { body }.insert(cx.tx()).await?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_hooks_without_a_connection_use_the_ordinary_client() {
	any_spawner::Executor::init_tokio().expect("Tokio executor");
	let db = leptoskit_db::testing::sqlite().await;

	tokio::task::LocalSet::new()
		.run_until(async {
			let owner = Owner::new();
			owner.set();
			leptoskit_query::QueryClient::new().provide();
			rpc::server::Server::new(db.clone()).install();

			let bodies = rpc::use_live(|| OfflineBodies {});
			tokio::time::timeout(std::time::Duration::from_secs(10), async {
				while bodies.data().is_none() {
					tokio::task::yield_now().await;
				}
			})
			.await
			.expect("the rows arrive");
			let send = rpc::use_live_mutation::<OfflineAdd>();
			send.mutate_async(OfflineAdd {
				body: "offline".into(),
			})
			.await
			.unwrap();
			let notes = OfflineNote::query().all(&db).await.unwrap();
			assert_eq!(notes.len(), 1);
			assert_eq!(notes[0].body, "offline");
		})
		.await;
}
