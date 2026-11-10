//! Runs the workloads that match Exact's published benchmarks.
//!
//! ```sh
//! cargo run --release -p exact_bench
//! ```
//!
//! Exact records presented frames on an iPhone, an iPad, and Chrome. This
//! binary records how long leptoskit takes to build and commit the same
//! window on the headless renderer. The two numbers answer different
//! questions; the comparison is written up next to Exact's published tables.

mod measure;
mod work;

use measure::measure;
use measure::print_row;
use work::crypto_at;
use work::crypto_live;
use work::extra_at;
use work::extra_live;
use work::heavy_at;
use work::heavy_live;
use work::list_at;
use work::list_live;
use work::markdown_at;
use work::markdown_live;

fn main() {
	println!(
		"leptoskit headless windows for Exact's workloads.\njump and step include building the \
		 next window and committing it.\ncommits/s is 1000/step, a host prepare rate, not a \
		 presented frame rate.\n"
	);
	println!(
		"| workload | rows | nodes | build ms | layout ms | jump p50 ms | step p50 ms | commits/s \
		 | live ms | rss KiB |"
	);
	println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
	for report in [
		measure("list", 10_000, 72.0, list_at, list_live),
		measure("heavy-list", 10_000, 120.0, heavy_at, heavy_live),
		measure("crypto-list", 5_000, 56.0, crypto_at, crypto_live),
		measure("extra-heavy", 3_000, 88.0, extra_at, extra_live),
		measure("markdown", 200, 64.0, markdown_at, markdown_live),
		measure("dioxus-feed", 10_000, 120.0, heavy_at, heavy_live),
		measure("web-shootout", 10_000, 120.0, heavy_at, heavy_live),
	] {
		print_row(&report);
	}
	println!("\nboard: list, heavy-list, and crypto-list are the three series Exact's board runs.");
}
