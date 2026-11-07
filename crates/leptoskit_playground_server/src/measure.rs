//! Ten uncached container compiles; run after building the toolchain image.
use std::collections::HashSet;
use std::time::Instant;

use base64::Engine as _;
use leptoskit_playground_server::Config;
use leptoskit_playground_server::run;
use sha2::Digest as _;
use sha2::Sha256;

const RUNS: usize = 10;
const LABEL: &str = "A small idea starts here";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	let config = Config::from_env()?;
	let source_path = std::env::args()
		.nth(1)
		.ok_or_else(|| anyhow::anyhow!("usage: playground-measure source.rs"))?;
	let source = std::fs::read_to_string(source_path)?;
	let mut samples = Vec::with_capacity(RUNS);
	let mut artifacts = HashSet::with_capacity(RUNS);
	let mut sizes = (0, 0);

	for iteration in 0..RUNS {
		let edited = source.replacen(LABEL, &format!("{LABEL} {iteration}"), 1);
		anyhow::ensure!(
			edited != source,
			"source must contain the counter starter label"
		);
		let began = Instant::now();
		let result = run(&config, &config.image, &edited).await?;
		let elapsed = began.elapsed().as_secs_f64();
		anyhow::ensure!(!result.wasm.is_empty(), "{}", result.stderr);
		let wasm = base64::engine::general_purpose::STANDARD.decode(&result.wasm)?;
		anyhow::ensure!(
			artifacts.insert(Sha256::digest(&wasm)),
			"distinct visible edits must produce distinct Wasm bundles"
		);
		sizes = (result.js.len(), wasm.len());
		println!(
			"{}",
			serde_json::json!({"iteration": iteration + 1, "seconds":elapsed})
		);
		samples.push(elapsed);
	}

	samples.sort_by(f64::total_cmp);
	println!(
		"{}",
		serde_json::json!({
			"runs": RUNS,
			"p50_seconds": samples[RUNS / 2 - 1],
			"p95_seconds": samples[RUNS - 1],
			"js_bytes": sizes.0,
			"wasm_bytes": sizes.1,
		})
	);
	Ok(())
}
