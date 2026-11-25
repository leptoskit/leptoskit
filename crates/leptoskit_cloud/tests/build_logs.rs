//! A build's log is charged what the platform keeps for it, so lines with
//! no bytes of their own are never kept for nothing, and a build's log has
//! at most `MAX_BUILD_LOG_CHUNKS` chunks, whatever their bytes.

#![cfg(not(target_arch = "wasm32"))]

mod support;

use std::path::Path;

use leptoskit_cloud::tables::Job;
use leptoskit_cloud::tables::LogChunk;
use leptoskit_cloud::tables::LogKind;
use leptoskit_cloud::tables::LogUsage;
use leptoskit_cloud::tables::NewLogChunk;
use leptoskit_cloud::workers::JobKind;
use leptoskit_cloud::workers::MAX_BUILD_LOG_CHUNKS;
use serde_json::json;
use support::App;
use support::shipping::Shop;

/// The build job, the one the scenario queued.
async fn the_job(app: &App) -> Job {
	Job::query()
		.filter(|job| job.kind.eq(JobKind::BuildServer))
		.one(&app.system())
		.await
		.expect("the build job")
}

/// How many chunks the index holds of the build job's log, over all its
/// attempts.
async fn chunks(app: &App) -> u64 {
	let job = the_job(app).await;

	LogChunk::query()
		.filter(|chunk| chunk.job_id.eq(Some(job.id)))
		.count(&app.system())
		.await
		.expect("the job's chunks")
}

/// What the build job's log has cost: the build's count, its chunk rows'
/// bytes together, and its organisation's count.
async fn charged(app: &App) -> [i64; 3] {
	let job = the_job(app).await;
	let rows = LogChunk::query()
		.filter(|chunk| chunk.job_id.eq(Some(job.id)))
		.all(&app.system())
		.await
		.expect("the job's chunks");
	let organisation = LogUsage::query()
		.filter(|usage| usage.organization_id.eq(job.organization_id))
		.first(&app.system())
		.await
		.expect("the organisation's usage");

	[
		job.log_bytes,
		rows.iter().map(|chunk| chunk.bytes).sum(),
		organisation.map_or(0, |usage| usage.bytes),
	]
}

/// How many objects the local store keeps of logs, and their bytes
/// together: every file under its `logs` bucket.
fn kept(app: &App) -> (usize, i64) {
	fn under(directory: &Path, objects: &mut Vec<u64>) {
		for entry in std::fs::read_dir(directory)
			.expect("the store's logs")
			.flatten()
		{
			let path = entry.path();

			if path.is_dir() {
				under(&path, objects);
			} else {
				objects.push(entry.metadata().expect("an object").len());
			}
		}
	}

	let mut objects = Vec::new();
	under(&app.storage.path().join("logs"), &mut objects);

	(
		objects.len(),
		i64::try_from(objects.iter().sum::<u64>()).expect("a size"),
	)
}

/// Puts `count` chunks of the build job's log straight into the index,
/// standing in for as many as its worker sent: the bound is on rows, and
/// their objects aren't written.
async fn indexed(app: &App, count: u64) {
	let job = the_job(app).await;

	app.system()
		.transaction(async |tx| {
			for seq in 0..count {
				NewLogChunk {
					kind: LogKind::Build,
					organization_id: job.organization_id,
					environment_id: job.environment_id.expect("a build job's environment"),
					deployment_id: job.deployment_id,
					job_id: Some(job.id),
					attempt: job.attempt,
					seq: i64::try_from(seq).expect("a seq"),
					first_line: i64::try_from(seq).expect("a line") + 1,
					lines: 1,
					bytes: 45,
					digest: format!("{seq:064x}"),
					object_key: format!("logs/standing-in/{seq}"),
					expires_at: leptoskit_db::now() + chrono::TimeDelta::days(30),
				}
				.insert(tx)
				.await?;
			}

			Ok::<_, leptoskit_db::DbError>(())
		})
		.await
		.expect("the chunks are indexed");
}

/// A chunk of empty lines has no bytes of its own, but is kept as JSON
/// lines: it is charged what the store keeps, so the caps hold however
/// little text a worker sends.
#[tokio::test]
async fn blank_log_lines_are_charged_what_the_store_keeps() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	// Two chunks of 1,000 empty lines and a small one fit under the caps,
	// and a third such chunk doesn't.
	let app = support::app_with(db, |config| {
		config.shipping.build_log_cap = 100_000;
		config.shipping.organization_log_cap = 100_000;
	})
	.await;
	let shop = Shop::open(&app).await;
	let digest = shop.upload(b"a quiet build").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	let blank: Vec<&str> = vec![""; 1_000];

	// 1,000 empty lines are kept as 1,000 JSON lines, some 44,000 bytes:
	// that is what the build and its organisation are charged.
	shop.build.log(&build, 0, &blank).await.expect(200);
	let stored = kept(&app);
	assert_eq!(stored.0, 1);
	assert_eq!(charged(&app).await, [stored.1; 3]);

	// A chunk of real lines is taken as it always was, and charged the same
	// way.
	shop.build
		.log(&build, 1, &["compiling", "done"])
		.await
		.expect(200);
	shop.build.log(&build, 2, &blank).await.expect(200);
	let stored = kept(&app);
	assert_eq!(stored.0, 3);
	assert_eq!(charged(&app).await, [stored.1; 3]);

	// Charged, blank lines fill the caps: a third such chunk is past them,
	// and nothing more is kept or charged.
	shop.build
		.log(&build, 3, &blank)
		.await
		.refused(409, "log_full");
	assert_eq!(kept(&app), stored);
	assert_eq!(charged(&app).await, [stored.1; 3]);
}

/// Every chunk is an index row and an object, whatever its bytes: a build's
/// log has at most `MAX_BUILD_LOG_CHUNKS` chunks, over all its attempts.
#[tokio::test]
async fn a_builds_log_has_at_most_so_many_chunks() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let digest = shop.upload(b"a chatty build").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	let bound = u32::try_from(MAX_BUILD_LOG_CHUNKS).expect("a number of chunks");

	// All but one of them are in the index already.
	indexed(&app, u64::from(bound - 1)).await;

	// The last chunk the bound allows is taken.
	shop.build.log(&build, bound - 1, &[""]).await.expect(200);
	assert_eq!(chunks(&app).await, u64::from(bound));
	let objects = kept(&app).0;

	// The chunk past the bound is refused, and nothing more is kept.
	shop.build
		.log(&build, bound, &[""])
		.await
		.refused(409, "log_full");
	assert_eq!(chunks(&app).await, u64::from(bound));
	assert_eq!(kept(&app).0, objects);

	// A chunk sent again is the one the store already has.
	shop.build.log(&build, bound - 1, &[""]).await.expect(200);
	assert_eq!(kept(&app).0, objects);

	// The bound covers a build's later attempts.
	shop.lapse_leases().await;
	assert_eq!(shop.sweep().await.retried, 1);
	let again = shop.build.take().await;
	assert_eq!(again["attempt"], 2);
	shop.build
		.log(&again, 0, &[""])
		.await
		.refused(409, "log_full");
	assert_eq!(kept(&app).0, objects);
	assert_eq!(chunks(&app).await, u64::from(bound));
}
