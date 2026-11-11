//! Timing for one virtualized workload on the headless renderer.

use std::time::Instant;

use leptoskit::native::Element;
use leptoskit::native::HeadlessBackend;
use leptoskit::native::NativeSurface;

/// What one workload cost on this machine.
pub struct Report {
	/// Benchmark name, matching Exact's directory.
	pub name: &'static str,
	/// Rows in the whole list.
	pub count: usize,
	/// Nodes in the window that was laid out.
	pub window_nodes: usize,
	/// Milliseconds to build that window's tree.
	pub build_ms: f64,
	/// Milliseconds for the headless renderer to commit it.
	pub layout_ms: f64,
	/// Median milliseconds to build and commit a jump.
	pub jump_p50_ms: f64,
	/// Median milliseconds to build and commit one scroll step.
	pub step_p50_ms: f64,
	/// `1000 / step_p50_ms`: how often this host could prepare a new window.
	pub commits_per_s: f64,
	/// Milliseconds to rebuild the window after a live change.
	pub live_ms: f64,
	/// Resident set after the run, in KiB, on Linux.
	pub rss_kb: u64,
}

/// Builds a window at a scroll offset.
pub type At = fn(f64) -> Element;

/// A live change, given a generation counter.
pub type Live = fn(f64, u32) -> Element;

/// Times the first window, ten jumps, and a short fling.
pub fn measure(name: &'static str, count: usize, row: f64, at: At, live: Live) -> Report {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(390.0, 844.0);

	let built_at = Instant::now();
	let first = at(0.0);
	let build_ms = millis(built_at);
	let window_nodes = count_nodes(&first);
	let laid_at = Instant::now();
	surface.set_root(&first);
	let layout_ms = millis(laid_at);

	let span = row * count as f64;
	let offsets: Vec<f64> = (1..=10).map(|step| span * f64::from(step) / 11.0).collect();
	let jump_p50_ms = percentile(at, &offsets);
	let step_p50_ms = fling(at, span);
	let live_at = Instant::now();
	surface.set_root(&live(0.0, 1));
	let live_ms = millis(live_at);

	Report {
		name,
		count,
		window_nodes,
		build_ms,
		layout_ms,
		jump_p50_ms,
		step_p50_ms,
		commits_per_s: if step_p50_ms > 0.0 {
			1000.0 / step_p50_ms
		} else {
			0.0
		},
		live_ms,
		rss_kb: rss_kb(),
	}
}

/// Prints one report as a Markdown table row.
pub fn print_row(report: &Report) {
	println!(
		"| {} | {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.0} | {:.3} | {} |",
		report.name,
		report.count,
		report.window_nodes,
		report.build_ms,
		report.layout_ms,
		report.jump_p50_ms,
		report.step_p50_ms,
		report.commits_per_s,
		report.live_ms,
		report.rss_kb
	);
}

fn percentile(at: impl Fn(f64) -> Element, offsets: &[f64]) -> f64 {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(390.0, 844.0);
	surface.set_root(&at(0.0));
	let mut samples = Vec::with_capacity(offsets.len());
	for offset in offsets {
		let started = Instant::now();
		let element = at(*offset);
		surface.set_root(&element);
		samples.push(millis(started));
	}
	median(&mut samples)
}

fn fling(at: At, span: f64) -> f64 {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(390.0, 844.0);
	surface.set_root(&at(0.0));
	let mut offset: f64 = 0.0;
	let mut samples = Vec::with_capacity(60);
	for speed in [1_000.0, 3_000.0, 6_000.0, 12_000.0, 24_000.0] {
		let delta = speed / 120.0;
		for _ in 0..12 {
			offset = (offset + delta).min(span);
			let started = Instant::now();
			let element = at(offset);
			surface.set_root(&element);
			samples.push(millis(started));
		}
	}
	median(&mut samples)
}

fn median(samples: &mut [f64]) -> f64 {
	samples.sort_by(f64::total_cmp);
	let mid = samples.len() / 2;
	if samples.len().is_multiple_of(2) {
		f64::midpoint(samples[mid - 1], samples[mid])
	} else {
		samples[mid]
	}
}

fn millis(started: Instant) -> f64 {
	started.elapsed().as_secs_f64() * 1_000.0
}

fn count_nodes(element: &Element) -> usize {
	1 + element.children.iter().map(count_nodes).sum::<usize>()
}

fn rss_kb() -> u64 {
	let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
		return 0;
	};
	status
		.lines()
		.find_map(|line| line.strip_prefix("VmRSS:"))
		.and_then(|rest| rest.split_whitespace().next())
		.and_then(|kb| kb.parse().ok())
		.unwrap_or(0)
}
