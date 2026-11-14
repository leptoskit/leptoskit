//! Charts: paths computed in Rust and drawn as `Vector`s, with labels as
//! text laid out around the plot, so they stay `text-xs` at any width.

use leptoskit::native::vector::Paint;
use leptoskit::native::vector::PathData;
use leptoskit::native::vector::Point;
use leptoskit::native::vector::Stroke;
use leptoskit::native::vector::StrokeScaling;
use leptoskit::native::vector::VectorPath;
use leptoskit::native::vector::VectorScaling;
use leptoskit::native::vector::ViewBox;
use leptoskit::prelude::*;

use crate::style;

/// One line on a chart.
pub(crate) struct Series {
	pub name: String,
	/// The palette slot, from 1.
	pub slot: usize,
	/// A value per x position; `None` where there's no observation.
	pub values: Vec<Option<f64>>,
	pub dashed: bool,
}

/// A labelled vertical marker: a release reaching the fleet.
pub(crate) struct Marker {
	pub at: usize,
	pub label: String,
}

/// What a chart draws.
pub(crate) struct LineChart {
	pub series: Vec<Series>,
	/// A horizontal threshold, such as an SLO, with its label.
	pub threshold: Option<(f64, String)>,
	pub markers: Vec<Marker>,
	/// The x labels at the start, the middle, and the end.
	pub x_labels: [String; 3],
	/// How y values print: `250 ms`, `1.2%`.
	pub format: fn(f64) -> String,
}

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 200.0;

fn stroke(width: f32) -> Stroke {
	Stroke::new(Paint::CurrentColor, width)
		.expect("a positive width")
		.round()
		.scaling(StrokeScaling::Fixed)
}

/// A rounded maximum for the y axis. Its thirds are the gridlines, so it
/// divides by three into tidy values: 300 gives 100, 200, 300.
fn nice_ceiling(value: f64) -> f64 {
	if value <= 0.0 {
		return 3.0;
	}

	let magnitude = 10f64.powf(value.log10().floor());

	for step in [1.5, 3.0, 4.5, 6.0, 9.0] {
		if value <= step * magnitude {
			return step * magnitude;
		}
	}

	15.0 * magnitude
}

#[expect(
	clippy::cast_possible_truncation,
	reason = "chart coordinates are small and only need f32 precision"
)]
fn y_of(value: f64, ceiling: f64) -> f32 {
	HEIGHT - (value / ceiling) as f32 * HEIGHT
}

#[expect(
	clippy::cast_precision_loss,
	reason = "a chart has at most a few hundred points"
)]
fn x_of(index: usize, count: usize) -> f32 {
	if count <= 1 {
		WIDTH / 2.0
	} else {
		index as f32 / (count - 1) as f32 * WIDTH
	}
}

/// A line chart: four gridlines, y labels on the left, x labels at the
/// start, middle, and end, a direct label per line, and markers.
#[expect(
	clippy::too_many_lines,
	reason = "one chart's layers in drawing order: grid, threshold, series, markers, labels, \
	          legend"
)]
pub(crate) fn line_chart(chart: &LineChart, label: &str) -> Element {
	let count = chart
		.series
		.iter()
		.map(|series| series.values.len())
		.max()
		.unwrap_or(0);
	let highest = chart
		.series
		.iter()
		.flat_map(|series| series.values.iter().flatten().copied())
		.chain(chart.threshold.as_ref().map(|(value, _)| *value))
		.fold(0.0_f64, f64::max);
	let ceiling = nice_ceiling(highest);
	let view_box = ViewBox::new(0.0, 0.0, WIDTH, HEIGHT).expect("a valid view box");

	let mut grid = Vector(view_box)
		.scaling(VectorScaling::Stretch)
		.class("absolute inset-0 text-[var(--admin-line)]")
		.aria_hidden(true);

	for step in 0..4 {
		#[expect(clippy::cast_precision_loss, reason = "four gridlines")]
		let y = HEIGHT - HEIGHT * (step as f32) / 3.0;
		if let Ok(data) = PathData::line(Point { x: 0.0, y }, Point { x: WIDTH, y }) {
			grid = grid.path(VectorPath::new(data).stroke(stroke(1.0)));
		}
	}

	let mut plot = View()
		.class("relative w-full aspect-[3/1] max-sm:aspect-[2/1]")
		.child(grid);

	for marker in &chart.markers {
		let x = x_of(marker.at, count);
		let percent = (f64::from(x) / f64::from(WIDTH) * 1000.0).round() / 10.0;
		plot = plot.child(
			View()
				.class("absolute inset-y-0 w-px bg-[var(--admin-ink-2)]")
				.class(format!("left-[{percent}%]"))
				.test_id("release-marker")
				.aria_label(format!("Release {}", marker.label)),
		);
		plot = plot.child(
			Text(marker.label.clone())
				.class(
					"absolute top-0 ml-1 rounded bg-[var(--admin-surface)] px-1 text-[11px] \
					 font-semibold text-[var(--admin-ink-2)]",
				)
				.class(format!("left-[{percent}%]"))
				.aria_hidden(true),
		);
	}

	if let Some((value, _)) = &chart.threshold {
		let y = y_of(*value, ceiling);
		if let (Ok(data), Ok(dashed)) = (
			PathData::line(Point { x: 0.0, y }, Point { x: WIDTH, y }),
			stroke(1.5).dashes(&[4.0, 4.0], 0.0),
		) {
			plot = plot.child(
				Vector(view_box)
					.scaling(VectorScaling::Stretch)
					.class("absolute inset-0 text-[var(--admin-ink-3)]")
					.aria_hidden(true)
					.path(VectorPath::new(data).stroke(dashed)),
			);
		}
	}

	for series in &chart.series {
		let mut vector = Vector(view_box)
			.scaling(VectorScaling::Stretch)
			.class("absolute inset-0")
			.class(style::series(series.slot))
			.aria_hidden(true);
		// Runs of observed points become separate lines, so a gap stays a gap.
		let mut run: Vec<Point> = Vec::new();
		let mut runs: Vec<Vec<Point>> = Vec::new();

		for (index, value) in series.values.iter().enumerate() {
			match value {
				Some(value) => {
					run.push(Point {
						x: x_of(index, count),
						y: y_of(*value, ceiling),
					});
				}
				None if !run.is_empty() => runs.push(std::mem::take(&mut run)),
				None => {}
			}
		}

		if !run.is_empty() {
			runs.push(run);
		}

		for run in runs {
			let line = if series.dashed {
				stroke(2.0)
					.dashes(&[4.0, 4.0], 0.0)
					.unwrap_or_else(|_| stroke(2.0))
			} else {
				stroke(2.0)
			};
			let data = if run.len() == 1 {
				PathData::circle(run[0].x, run[0].y, 2.5)
			} else {
				PathData::polyline(run)
			};

			if let Ok(data) = data {
				vector = vector.path(VectorPath::new(data).stroke(line));
			}
		}

		plot = plot.child(vector);
	}

	let y_axis = View()
		.class("w-14 shrink-0 flex-col justify-between items-end pr-2")
		.aria_hidden(true)
		.children((0..4).rev().map(|step| {
			Text((chart.format)(ceiling * f64::from(step) / 3.0))
				.class("text-[11px] tabular-nums text-[var(--admin-ink-3)] leading-none")
		}));

	let mut legend = View().class("flex-row flex-wrap gap-x-4 gap-y-1");

	for series in &chart.series {
		let last = series
			.values
			.iter()
			.rev()
			.flatten()
			.next()
			.map_or_else(|| "no data".to_owned(), |value| (chart.format)(*value));
		legend = legend.child(
			View()
				.class("flex-row items-center gap-1.5")
				.child(
					View()
						.class("h-0.5 w-4 rounded-full")
						.class(series_fill(series.slot)),
				)
				.child(
					Text(format!("{} · {last}", series.name))
						.class("text-xs text-[var(--admin-ink-2)]"),
				),
		);
	}

	if let Some((value, name)) = &chart.threshold {
		legend = legend.child(
			View()
				.class("flex-row items-center gap-1.5")
				.child(View().class("h-0.5 w-4 bg-[var(--admin-ink-3)]"))
				.child(
					Text(format!("{name} · {}", (chart.format)(*value)))
						.class("text-xs text-[var(--admin-ink-2)]"),
				),
		);
	}

	View()
		.class("flex-col gap-2")
		.role(leptoskit::native::Role::Img)
		.aria_label(label)
		.child(legend)
		.child(
			View()
				.class("flex-row")
				.child(y_axis)
				.child(plot.class("flex-1")),
		)
		.child(
			View()
				.class("flex-row justify-between pl-14")
				.aria_hidden(true)
				.children(
					chart
						.x_labels
						.iter()
						.map(|label| Text(label.clone()).class(style::LABEL)),
				),
		)
}

fn series_fill(slot: usize) -> &'static str {
	match slot {
		1 => "bg-[var(--admin-series-1)]",
		2 => "bg-[var(--admin-series-2)]",
		3 => "bg-[var(--admin-series-3)]",
		4 => "bg-[var(--admin-series-4)]",
		5 => "bg-[var(--admin-series-5)]",
		6 => "bg-[var(--admin-series-6)]",
		_ => "bg-[var(--admin-ink-3)]",
	}
}

/// A sparkline: a thin line, no axes, the last point as a dot.
pub(crate) fn sparkline(values: &[f64]) -> Element {
	let view_box = ViewBox::new(0.0, 0.0, 100.0, 24.0).expect("a valid view box");
	let mut vector = Vector(view_box)
		.scaling(VectorScaling::Stretch)
		.class("h-6 w-24 text-[var(--admin-ink-2)]")
		.aria_hidden(true);
	let highest = values
		.iter()
		.copied()
		.fold(0.0_f64, f64::max)
		.max(f64::EPSILON);
	#[expect(
		clippy::cast_possible_truncation,
		clippy::cast_precision_loss,
		reason = "a sparkline has a few dozen points"
	)]
	let points: Vec<Point> = values
		.iter()
		.enumerate()
		.map(|(index, value)| {
			Point {
				x: if values.len() <= 1 {
					50.0
				} else {
					index as f32 / (values.len() - 1) as f32 * 100.0
				},
				y: 22.0 - (value / highest) as f32 * 20.0,
			}
		})
		.collect();

	if points.len() > 1
		&& let Ok(data) = PathData::polyline(points.clone())
	{
		vector = vector.path(VectorPath::new(data).stroke(stroke(1.5)));
	}

	vector
}

/// `250 ms`, `1.2 s`.
pub(crate) fn milliseconds(value: f64) -> String {
	if value >= 1000.0 {
		format!("{:.1} s", value / 1000.0)
	} else {
		format!("{value:.0} ms")
	}
}

/// `1.2%`.
pub(crate) fn percent(value: f64) -> String {
	format!("{value:.1}%")
}

/// `12.4k`.
#[expect(clippy::cast_precision_loss, reason = "display only")]
pub(crate) fn count(value: u64) -> String {
	match value {
		0..1000 => value.to_string(),
		1000..1_000_000 => format!("{:.1}k", value as f64 / 1000.0),
		_ => format!("{:.1}M", value as f64 / 1_000_000.0),
	}
}

/// `512 MB`, `1.5 GB`.
#[expect(clippy::cast_precision_loss, reason = "display only")]
pub(crate) fn bytes(value: u64) -> String {
	const MB: f64 = 1024.0 * 1024.0;
	let megabytes = value as f64 / MB;

	if megabytes >= 1024.0 {
		format!("{:.1} GB", megabytes / 1024.0)
	} else {
		format!("{megabytes:.0} MB")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ceilings_are_tidy() {
		assert!((nice_ceiling(0.0) - 3.0).abs() < f64::EPSILON);
		assert!((nice_ceiling(180.0) - 300.0).abs() < f64::EPSILON);
		assert!((nice_ceiling(500.0) - 600.0).abs() < f64::EPSILON);
		assert!((nice_ceiling(7.0) - 9.0).abs() < f64::EPSILON);
	}

	#[test]
	fn formats_read_plainly() {
		assert_eq!(milliseconds(250.0), "250 ms");
		assert_eq!(milliseconds(1250.0), "1.2 s");
		assert_eq!(count(12_400), "12.4k");
		assert_eq!(bytes(512 * 1024 * 1024), "512 MB");
	}
}
