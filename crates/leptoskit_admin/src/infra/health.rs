//! Health: what the fleet's servers reported in the last hour, merged
//! across instances by the control service.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use leptoskit::apm::LATENCY_BOUNDS;
use leptoskit::apm::Outcome;
use leptoskit::apm::Signal;
use leptoskit::apm::Source;
use leptoskit::apm::Window;
use leptoskit::prelude::*;
use leptoskit_deploy_console::control::Fleet;

use crate::charts;
use crate::charts::LineChart;
use crate::charts::Marker;
use crate::charts::Series;
use crate::components;
use crate::components::Tone;
use crate::style;

/// Merged counts for one window or one route.
#[derive(Clone, Debug, Default)]
struct Merged {
	count: u64,
	errors: u64,
	latency: [u64; 12],
}

impl Merged {
	fn add(&mut self, window: &Window, error: bool) {
		self.count += window.count;

		if error {
			self.errors += window.count;
		}

		for (total, bucket) in self.latency.iter_mut().zip(window.latency) {
			*total += bucket;
		}
	}

	/// The upper bound of the bucket holding the `rank`th observation of
	/// `count`, as the collector's own `p95_upper_ms` reads it.
	fn quantile(&self, keep: u64) -> Option<u64> {
		if self.count == 0 {
			return None;
		}

		let rank = self.count - self.count / keep;
		let mut total = 0;

		for (index, bucket) in self.latency.iter().enumerate() {
			total += bucket;

			if total >= rank {
				return Some(LATENCY_BOUNDS.get(index).copied().unwrap_or(10_000));
			}
		}

		None
	}

	fn p95(&self) -> Option<u64> {
		self.quantile(20)
	}

	fn p50(&self) -> Option<u64> {
		self.quantile(2)
	}

	#[expect(clippy::cast_precision_loss, reason = "a rate for display")]
	fn error_rate(&self) -> Option<f64> {
		(self.count > 0).then(|| self.errors as f64 / self.count as f64 * 100.0)
	}
}

/// One route's row.
struct Route {
	name: String,
	merged: Merged,
	per_window: Vec<f64>,
}

/// The health page's content.
#[expect(clippy::cast_precision_loss, reason = "rates for display")]
#[expect(
	clippy::too_many_lines,
	reason = "the health tab in reading order: tiles, charts, routes, hosts, limits"
)]
pub(crate) fn section(fleet: &Fleet, slo_ms: u64) -> Vec<Element> {
	let http = |source: Source, signal: Signal| source == Source::Backend && signal == Signal::Http;
	let series: Vec<_> = fleet
		.series
		.iter()
		.filter(|series| http(series.key.source, series.key.signal))
		.collect();

	if series.is_empty() {
		return vec![components::empty_state(
			"No requests reported yet",
			"Health shows what your servers report: requests, latency, errors, and each host's \
			 memory and disk, merged across instances. Servers start reporting once the app is \
			 built with the apm feature and [infra] turns the analytics plugin on. Add this, \
			 plan, apply, and deploy:",
			Some((
				"[infra.plugins.analytics]\nversion = 1\nenvironment = \"production\"",
				"TOML",
			)),
			"The first numbers appear here a minute after the first request.",
		)];
	}

	let starts: BTreeSet<u64> = series
		.iter()
		.flat_map(|series| {
			series
				.windows
				.iter()
				.map(|window| window.start_unix_seconds)
		})
		.collect();
	let starts: Vec<u64> = starts.into_iter().collect();
	let position: BTreeMap<u64, usize> = starts
		.iter()
		.enumerate()
		.map(|(index, start)| (*start, index))
		.collect();
	let mut windows = vec![Merged::default(); starts.len()];
	let mut routes: BTreeMap<String, Route> = BTreeMap::new();
	let mut releases: Vec<BTreeSet<String>> = vec![BTreeSet::new(); starts.len()];
	let mut total = Merged::default();

	for series in &series {
		let error = series.key.outcome == Outcome::ServerError;
		let route = routes
			.entry(series.key.operation.clone())
			.or_insert_with(|| {
				Route {
					name: series.key.operation.clone(),
					merged: Merged::default(),
					per_window: vec![0.0; starts.len()],
				}
			});

		for window in &series.windows {
			let index = position[&window.start_unix_seconds];
			windows[index].add(window, error);
			route.merged.add(window, error);
			route.per_window[index] += window.count as f64;
			total.add(window, error);

			if window.count > 0 {
				releases[index].insert(series.release.clone());
			}
		}
	}

	let minutes = (starts.len().max(1)) as f64;
	let tiles = View()
		.class("grid grid-cols-4 gap-4 max-lg:grid-cols-2")
		.test_id("grid-tiles")
		.child(tile(
			"Requests",
			&format!(
				"{}/min",
				charts::count((total.count as f64 / minutes).round() as u64)
			),
			&windows.iter().map(|w| w.count as f64).collect::<Vec<_>>(),
		))
		.child(tile(
			"p95 latency",
			&total
				.p95()
				.map_or_else(|| "—".to_owned(), |p95| charts::milliseconds(p95 as f64)),
			&windows
				.iter()
				.map(|w| w.p95().unwrap_or(0) as f64)
				.collect::<Vec<_>>(),
		))
		.child(tile(
			"Error rate",
			&total
				.error_rate()
				.map_or_else(|| "—".to_owned(), charts::percent),
			&windows
				.iter()
				.map(|w| w.error_rate().unwrap_or(0.0))
				.collect::<Vec<_>>(),
		))
		.child(tile(
			"Instances",
			&if fleet.stale_instances > 0 {
				format!("{} ({} quiet)", fleet.instances, fleet.stale_instances)
			} else {
				fleet.instances.to_string()
			},
			&[],
		));

	let mut markers = Vec::new();
	let mut previous: Option<&BTreeSet<String>> = None;

	for (index, set) in releases.iter().enumerate() {
		if let Some(before) = previous
			&& let Some(new) = set.difference(before).next()
		{
			markers.push(Marker {
				at: index,
				label: crate::infra::short(new).to_owned(),
			});
		}

		if !set.is_empty() {
			previous = Some(set);
		}
	}

	let clock = |unix: u64| {
		let minutes = unix / 60 % (24 * 60);
		format!("{:02}:{:02}", minutes / 60, minutes % 60)
	};
	let x_labels = [
		starts
			.first()
			.map_or_else(String::new, |start| clock(*start)),
		starts
			.get(starts.len() / 2)
			.map_or_else(String::new, |start| clock(*start)),
		starts
			.last()
			.map_or_else(String::new, |start| clock(*start)),
	];

	let latency = LineChart {
		series: vec![
			Series {
				name: "p95".into(),
				slot: 1,
				values: windows.iter().map(|w| w.p95().map(|v| v as f64)).collect(),
				dashed: false,
			},
			Series {
				name: "p50".into(),
				slot: 2,
				values: windows.iter().map(|w| w.p50().map(|v| v as f64)).collect(),
				dashed: false,
			},
		],
		threshold: Some((slo_ms as f64, "SLO".to_owned())),
		markers,
		x_labels: x_labels.clone(),
		format: charts::milliseconds,
	};
	let errors = LineChart {
		series: vec![Series {
			name: "Server errors".into(),
			slot: 6,
			values: windows.iter().map(Merged::error_rate).collect(),
			dashed: false,
		}],
		threshold: None,
		markers: Vec::new(),
		x_labels,
		format: charts::percent,
	};

	let mut routes: Vec<Route> = routes.into_values().collect();
	routes.sort_by(|a, b| {
		b.merged
			.p95()
			.cmp(&a.merged.p95())
			.then_with(|| a.name.cmp(&b.name))
	});

	let mut table = View().class("flex-col").test_id("routes");
	table = table.child(
		View()
			.class("flex-row gap-3 border-b border-[var(--admin-line)] pb-2 max-sm:hidden")
			.child(Text("Route").class(style::LABEL).class("flex-1"))
			.child(
				Text("Requests/min")
					.class(style::LABEL)
					.class("w-28 text-end"),
			)
			.child(Text("p95").class(style::LABEL).class("w-20 text-end"))
			.child(Text("Errors").class(style::LABEL).class("w-20 text-end"))
			.child(View().class("w-24")),
	);

	for route in &routes {
		let rate = route.merged.error_rate().unwrap_or(0.0);
		table = table.child(
			View()
				.class(
					"flex-row flex-wrap items-center gap-x-3 gap-y-1 border-b \
					 border-[var(--admin-line)] py-2.5",
				)
				.test_id("route-row")
				.child(
					Text(route.name.clone())
						.class(style::CODE)
						.class("min-w-0 flex-1 truncate max-sm:basis-full"),
				)
				.child(
					Text(format!("{:.1}", route.merged.count as f64 / minutes)).class(
						"w-28 text-end text-sm tabular-nums max-sm:w-auto max-sm:text-start",
					),
				)
				.child(
					Text(
						route
							.merged
							.p95()
							.map_or_else(|| "—".to_owned(), |v| charts::milliseconds(v as f64)),
					)
					.class("w-20 text-end text-sm tabular-nums max-sm:w-auto"),
				)
				.child(
					Text(if rate > 0.0 {
						format!("! {}", charts::percent(rate))
					} else {
						charts::percent(rate)
					})
					.class("w-20 text-end text-sm tabular-nums max-sm:w-auto")
					.class(if rate > 1.0 {
						Tone::Warn.text()
					} else {
						"text-[var(--admin-ink)]"
					}),
				)
				.child(
					View()
						.class("w-24 max-sm:hidden")
						.child(charts::sparkline(&route.per_window)),
				),
		);
	}

	let mut hosts = View().class("grid grid-cols-2 gap-4 max-md:grid-cols-1");

	if fleet.hosts.is_empty() {
		hosts = hosts.child(
			Text("No host samples yet. Hosts report memory and disk alongside requests.")
				.class(style::MUTED),
		);
	}

	for host in &fleet.hosts {
		let mut card = View()
			.class("flex-col gap-3 rounded-[10px] border border-[var(--admin-line)] p-3")
			.test_id("host")
			.child(
				View()
					.class("flex-row items-end justify-between gap-2")
					.child(
						Text(host.instance.clone())
							.class(style::CODE)
							.class("truncate"),
					)
					.child(Text(crate::infra::short(&host.release).to_owned()).class(style::LABEL)),
			);

		match &host.host {
			Some(sample) => {
				let memory = sample.cgroup_memory_bytes.or(sample.process_resident_bytes);

				match (memory, sample.cgroup_memory_limit_bytes) {
					(Some(used), Some(limit)) if limit > 0 => {
						card = card.child(components::meter(
							"Memory",
							used as f64,
							limit as f64,
							&format!("{} of {}", charts::bytes(used), charts::bytes(limit)),
						));
					}
					(Some(used), _) => {
						card = card.child(components::fact(
							"Memory",
							Text(charts::bytes(used)).class(style::BODY),
						));
					}
					_ => {
						card = card.child(
							Text("Memory isn't reported on this platform.").class(style::MUTED),
						);
					}
				}

				if let (Some(available), Some(capacity)) = (
					sample.filesystem_available_bytes,
					sample.filesystem_capacity_bytes,
				) && capacity > 0
				{
					let used = capacity.saturating_sub(available);
					card = card.child(components::meter(
						"Disk",
						used as f64,
						capacity as f64,
						&format!("{} of {}", charts::bytes(used), charts::bytes(capacity)),
					));
				}
			}
			None => {
				card = card.child(Text("This host hasn't sent a sample yet.").class(style::MUTED));
			}
		}

		hosts = hosts.child(card);
	}

	let mut limitations = Vec::new();

	for limitation in &fleet.limitations {
		limitations.push(Text(limitation.clone()).class(style::MUTED));
	}

	vec![
		tiles,
		components::card("Latency")
			.aside("last hour · release markers show deploys")
			.child(charts::line_chart(
				&latency,
				"Latency: p95 and p50 per minute, with the SLO",
			))
			.build(),
		components::card("Error rate")
			.aside("5xx responses")
			.child(charts::line_chart(&errors, "Server error rate per minute"))
			.build(),
		components::card("Routes")
			.aside("sorted by p95")
			.child(table)
			.build(),
		components::card("Hosts")
			.child(hosts)
			.children(limitations)
			.build(),
	]
}

fn tile(label: &str, value: &str, spark: &[f64]) -> Element {
	let mut tile = View()
		.class(style::CARD)
		.class("gap-2")
		.test_id("stat")
		.child(Text(label.to_owned()).class(style::SECONDARY))
		.child(
			Text(value.to_owned())
				.class(style::HERO_NUMBER)
				.class("text-2xl"),
		);

	if spark.len() > 1 {
		tile = tile.child(charts::sparkline(spark));
	}

	tile
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn quantiles_read_bucket_bounds() {
		let mut merged = Merged::default();
		let mut window = Window {
			start_unix_seconds: 0,
			count: 100,
			..Window::default()
		};
		window.latency[2] = 90;
		window.latency[5] = 10;
		merged.add(&window, false);
		assert_eq!(merged.p50(), Some(25));
		assert_eq!(merged.p95(), Some(250));
		assert_eq!(merged.error_rate(), Some(0.0));
	}
}
