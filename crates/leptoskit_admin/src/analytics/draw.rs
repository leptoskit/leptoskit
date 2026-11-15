//! The analytics charts, from `docs/agents/admin-design.md` §6: a trend's
//! lines with the previous period dashed and a label at each line's end,
//! bar lists, funnel columns with the lost part hatched, the retention
//! grid, and sparklines.
//!
//! Plots are `Vector`s stretched over a box of fixed aspect; every label is
//! text laid out around them, so it stays `text-xs` at any width. Points
//! you can press are full-height columns over the plot, not the 2px line.

use leptoskit::native::Role;
use leptoskit::native::vector::Paint;
use leptoskit::native::vector::PathData;
use leptoskit::native::vector::Point;
use leptoskit::native::vector::Stroke;
use leptoskit::native::vector::StrokeScaling;
use leptoskit::native::vector::VectorPath;
use leptoskit::native::vector::VectorScaling;
use leptoskit::native::vector::ViewBox;
use leptoskit::prelude::*;

use super::format;
use crate::style;

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 240.0;

fn stroke(width: f32) -> Stroke {
	Stroke::new(Paint::CurrentColor, width)
		.expect("a positive width")
		.round()
		.scaling(StrokeScaling::Fixed)
}

fn dashed(width: f32) -> Stroke {
	stroke(width)
		.dashes(&[4.0, 4.0], 0.0)
		.unwrap_or_else(|_| stroke(width))
}

/// The background class for a series slot.
pub(crate) fn fill(slot: usize) -> &'static str {
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

/// Rounds a percentage for a class: `37.5`.
fn percent(fraction: f64) -> f64 {
	(fraction.clamp(0.0, 1.0) * 1000.0).round() / 10.0
}

/// A rounded maximum whose quarters are tidy: 2,000 gives 500, 1,000, …,
/// and 230 gives 300, so the axis reads 75, 150, 225 rather than 62.5.
fn ceiling(value: f64) -> f64 {
	if value <= 0.0 {
		return 4.0;
	}

	let magnitude = 10f64.powf(value.log10().floor());

	for step in [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 10.0] {
		if value <= step * magnitude {
			return step * magnitude;
		}
	}

	10.0 * magnitude
}

#[expect(
	clippy::cast_possible_truncation,
	reason = "chart coordinates only need f32 precision"
)]
fn y_of(value: f64, top: f64) -> f32 {
	HEIGHT - (value / top) as f32 * HEIGHT
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

fn plot_vector() -> Element {
	Vector(ViewBox::new(0.0, 0.0, WIDTH, HEIGHT).expect("a valid view box"))
		.scaling(VectorScaling::Stretch)
		.class("absolute inset-0")
		.aria_hidden(true)
}

/// One line of a trend.
pub(crate) struct Line {
	pub name: String,
	/// The palette slot, from 1.
	pub slot: usize,
	pub values: Vec<f64>,
	/// The previous period's values, bucket for bucket, when compared.
	pub previous: Option<Vec<f64>>,
}

impl Line {
	/// The previous period, unless it's empty: a flat dashed line at zero
	/// says nothing a missing one doesn't, and looks like data.
	fn previous(&self) -> Option<&Vec<f64>> {
		self.previous
			.as_ref()
			.filter(|values| values.iter().any(|value| *value > 0.0))
	}
}

/// A trend chart.
pub(crate) struct Trend {
	pub lines: Vec<Line>,
	/// Every bucket's label; the axis shows the first, middle, and last.
	pub labels: Vec<String>,
	/// Where pressing each bucket goes: the people behind it.
	pub targets: Vec<Option<String>>,
	/// Whether the last bucket is still filling, so it's drawn dashed.
	pub partial: bool,
	/// How values print.
	pub format: fn(f64) -> String,
	/// `aspect-[5/2]` for a standard card, `aspect-[4/1]` for a wide one.
	pub aspect: &'static str,
}

/// A line chart with gridlines, y labels, three x labels, the previous
/// period dashed in the same hue, an area under a lone line, and each
/// line's name and last value at its end.
#[expect(
	clippy::too_many_lines,
	reason = "one chart's layers in drawing order: grid, area, previous, lines, presses, labels"
)]
pub(crate) fn trend(chart: &Trend, label: &str) -> Element {
	let count = chart.labels.len().max(
		chart
			.lines
			.iter()
			.map(|line| line.values.len())
			.max()
			.unwrap_or(0),
	);
	let highest = chart
		.lines
		.iter()
		.flat_map(|line| {
			line.values
				.iter()
				.chain(line.previous().into_iter().flatten())
				.copied()
		})
		.fold(0.0_f64, f64::max);
	// Counts get whole-number gridlines: a day of one visitor reads 1, 2,
	// 3, 4 up the side, not 0.2, 0.5, 0.8.
	let counts = chart
		.lines
		.iter()
		.flat_map(|line| {
			line.values
				.iter()
				.chain(line.previous().into_iter().flatten())
		})
		.all(|value| value.fract() == 0.0);
	let top = if counts {
		(ceiling(highest) / 4.0).ceil() * 4.0
	} else {
		ceiling(highest)
	};

	let mut grid = plot_vector().class("text-[var(--admin-line)]");

	for step in 0..5 {
		#[expect(clippy::cast_precision_loss, reason = "five gridlines")]
		let y = HEIGHT - HEIGHT * step as f32 / 4.0;
		if let Ok(data) = PathData::line(Point { x: 0.0, y }, Point { x: WIDTH, y }) {
			grid = grid.path(VectorPath::new(data).stroke(stroke(1.0)));
		}
	}

	let mut plot = View()
		.class("relative min-w-0 flex-1")
		.class(chart.aspect)
		.child(grid);
	let points = |values: &[f64]| -> Vec<Point> {
		values
			.iter()
			.enumerate()
			.map(|(index, value)| {
				Point {
					x: x_of(index, count),
					y: y_of(*value, top),
				}
			})
			.collect()
	};

	if let [only] = chart.lines.as_slice()
		&& only.values.len() > 1
	{
		let mut area = points(&only.values);
		area.push(Point {
			x: x_of(only.values.len() - 1, count),
			y: HEIGHT,
		});
		area.push(Point { x: 0.0, y: HEIGHT });

		if let Ok(data) = PathData::polygon(area) {
			plot = plot.child(
				plot_vector()
					.class(style::series(only.slot))
					.class("opacity-[0.12]")
					.path(VectorPath::new(data).fill(Paint::CurrentColor)),
			);
		}
	}

	for line in &chart.lines {
		if let Some(previous) = line.previous()
			&& previous.len() > 1
			&& let Ok(data) = PathData::polyline(points(previous))
		{
			plot = plot.child(
				plot_vector()
					.class(style::series(line.slot))
					.class("opacity-60")
					.path(VectorPath::new(data).stroke(dashed(1.5))),
			);
		}

		let all = points(&line.values);
		let mut vector = plot_vector().class(style::series(line.slot));
		let solid = if chart.partial && all.len() > 2 {
			&all[..all.len() - 1]
		} else {
			&all[..]
		};

		match solid {
			[] => {}
			[point] => {
				if let Ok(data) = PathData::circle(point.x, point.y, 3.0) {
					vector = vector.path(VectorPath::new(data).fill(Paint::CurrentColor));
				}
			}
			_ => {
				if let Ok(data) = PathData::polyline(solid.to_vec()) {
					vector = vector.path(VectorPath::new(data).stroke(stroke(2.0)));
				}
			}
		}

		if chart.partial
			&& all.len() > 2
			&& let Ok(data) = PathData::line(all[all.len() - 2], all[all.len() - 1])
		{
			vector = vector.path(VectorPath::new(data).stroke(dashed(2.0)));
		}

		plot = plot.child(vector);
	}

	// Each bucket is a column you can press.
	if chart.targets.iter().any(Option::is_some) {
		let mut presses = View()
			.class("absolute inset-0 flex-row")
			.test_id("chart-points");

		for (index, target) in chart.targets.iter().enumerate() {
			let name = chart.labels.get(index).cloned().unwrap_or_default();
			presses = presses.child(match target {
				Some(href) => {
					Pressable(format!("Show the people behind {name}"))
						.href(href.clone())
						.class("h-full flex-1 cursor-pointer hover:bg-[var(--admin-ink)]/5")
						.test_id("chart-point")
				}
				None => View().class("h-full flex-1"),
			});
		}

		plot = plot.child(presses);
	}

	// Labels at the ends of the lines, pushed apart so they don't overlap.
	let mut ends: Vec<(f64, String, String, usize)> = chart
		.lines
		.iter()
		.filter_map(|line| {
			let last = *line.values.last()?;
			Some((
				f64::from(y_of(last, top) / HEIGHT),
				line.name.clone(),
				(chart.format)(last),
				line.slot,
			))
		})
		.collect();
	ends.sort_by(|a, b| a.0.total_cmp(&b.0));

	// One label line is about an eighth of a small chart's height. Push
	// each label below the one above it, then back up from the axis so the
	// lowest stays inside the plot.
	const GAP: f64 = 0.13;

	for index in 1..ends.len() {
		let floor = ends[index - 1].0 + GAP;
		if ends[index].0 < floor {
			ends[index].0 = floor;
		}
	}

	for index in (0..ends.len()).rev() {
		let ceiling = if index + 1 == ends.len() {
			0.97
		} else {
			ends[index + 1].0 - GAP
		};
		ends[index].0 = ends[index].0.min(ceiling).max(0.0);
	}

	let mut labels = View().class("relative w-24 shrink-0 max-sm:w-16");

	if chart.lines.len() <= 4 {
		for (at, name, value, slot) in ends {
			labels = labels.child(
				View()
					.class("absolute left-2 right-0 -mt-[7px] flex-row items-center gap-1")
					.class(format!("top-[{}%]", percent(at)))
					.child(
						Text(name)
							.class(
								"min-w-0 shrink truncate text-[11px] font-semibold leading-tight",
							)
							.class(style::series(slot)),
					)
					.child(Text(value).class(
						"shrink-0 text-[11px] leading-tight tabular-nums text-[var(--admin-ink-3)]",
					)),
			);
		}
	}

	let y_axis = View()
		.class("w-12 shrink-0 flex-col justify-between items-end pr-2")
		.class(chart.aspect)
		.aria_hidden(true)
		.children((0..5).rev().map(|step| {
			Text((chart.format)(top * f64::from(step) / 4.0))
				.class("-my-1.5 text-[11px] tabular-nums leading-none text-[var(--admin-ink-3)]")
		}));

	let mut legend = View().class("flex-row flex-wrap gap-x-4 gap-y-1");

	for line in &chart.lines {
		legend = legend.child(
			View()
				.class("flex-row items-center gap-1.5")
				.child(
					View()
						.class("h-0.5 w-4 rounded-full")
						.class(fill(line.slot)),
				)
				.child(Text(line.name.clone()).class("text-xs text-[var(--admin-ink-2)]")),
		);
	}

	if chart.lines.iter().any(|line| line.previous().is_some()) {
		legend = legend.child(
			View()
				.class("flex-row items-center gap-1.5")
				.child(
					View()
						.class("flex-row gap-0.5")
						.child(View().class("h-0.5 w-1.5 bg-[var(--admin-ink-3)]"))
						.child(View().class("h-0.5 w-1.5 bg-[var(--admin-ink-3)]")),
				)
				.child(Text("Previous period").class("text-xs text-[var(--admin-ink-2)]")),
		);
	}

	let axis = match chart.labels.as_slice() {
		[] => Vec::new(),
		[only] => vec![only.clone()],
		labels => {
			vec![
				labels[0].clone(),
				labels[labels.len() / 2].clone(),
				labels[labels.len() - 1].clone(),
			]
		}
	};

	View()
		.class("flex-col gap-3")
		.role(Role::Img)
		.aria_label(label)
		.test_id("trend-chart")
		.child(legend)
		.child(
			View()
				.class("flex-col gap-1.5")
				.child(
					View()
						.class("flex-row")
						.child(y_axis)
						.child(plot)
						.child(labels),
				)
				.child(
					View()
						.class("flex-row justify-between pl-12 pr-24 max-sm:pr-16")
						.aria_hidden(true)
						.children(
							axis.into_iter()
								.map(|label| Text(label).class(style::LABEL)),
						),
				),
		)
}

/// One row of a bar list.
pub(crate) struct Bar {
	pub label: String,
	pub value: f64,
	/// Where pressing the row goes: the page filtered by it.
	pub href: Option<String>,
}

/// Horizontal share bars behind text, value right-aligned.
pub(crate) fn bar_list(head: (&str, &str), rows: &[Bar], empty: &str) -> Element {
	let highest = rows.iter().map(|row| row.value).fold(0.0_f64, f64::max);
	let mut list = View().class("flex-col gap-1").child(
		View()
			.class("flex-row justify-between px-3 pb-1")
			.child(Text(head.0.to_owned()).class(style::LABEL))
			.child(Text(head.1.to_owned()).class(style::LABEL)),
	);

	if rows.is_empty() {
		return list.child(
			Text(empty.to_owned())
				.class(style::MUTED)
				.class("px-3 py-2"),
		);
	}

	for row in rows {
		let share = if highest > 0.0 {
			row.value / highest
		} else {
			0.0
		};
		let content = View()
			.class("relative flex-row items-center justify-between gap-3 rounded-lg px-3 py-1.5")
			.child(
				View()
					.class(
						"absolute inset-y-0 left-0 rounded-lg bg-[var(--admin-series-1)]/15 \
						 dark:bg-[var(--admin-series-1)]/25",
					)
					.class(format!("w-[{}%]", percent(share)))
					.aria_hidden(true),
			)
			.child(
				Text(if row.label.is_empty() {
					"(none)".to_owned()
				} else {
					row.label.clone()
				})
				.class("relative min-w-0 flex-1 truncate text-sm text-[var(--admin-ink)]"),
			)
			.child(
				Text(format::count(row.value))
					.class("relative text-sm font-semibold tabular-nums text-[var(--admin-ink)]"),
			);
		list = list.child(match &row.href {
			Some(href) => {
				Pressable(format!("Filter by {}", row.label))
					.href(href.clone())
					.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
					.test_id("bar-row")
					.child(content)
			}
			None => content,
		});
	}

	list
}

/// Diagonal lines in `--admin-line-strong`: the part of a funnel column
/// that dropped off.
fn hatch() -> Element {
	let mut vector = Vector(ViewBox::new(0.0, 0.0, 200.0, 200.0).expect("a valid view box"))
		.scaling(VectorScaling::Fill)
		.class("absolute inset-0 text-[var(--admin-line-strong)]")
		.aria_hidden(true);

	for step in 0..34 {
		#[expect(clippy::cast_precision_loss, reason = "a few dozen lines")]
		let offset = step as f32 * 12.0;
		if let Ok(data) = PathData::line(
			Point {
				x: offset - 200.0,
				y: 200.0,
			},
			Point { x: offset, y: 0.0 },
		) {
			vector = vector.path(VectorPath::new(data).stroke(stroke(1.0)));
		}
	}

	vector
}

/// One funnel step's column.
pub(crate) struct Step {
	pub name: String,
	pub count: u64,
	/// From the step before, 0 to 1.
	pub conversion: Option<f64>,
	/// The people who reached it.
	pub reached: Option<String>,
	/// The people who reached the step before and not this one.
	pub dropped: Option<String>,
}

/// Funnel columns, left to right, height the people at each step, the
/// lost part hatched, the conversion between them.
#[expect(
	clippy::cast_precision_loss,
	reason = "people counts become shares for drawing"
)]
pub(crate) fn funnel(steps: &[Step], height: &'static str) -> Element {
	let first = steps.first().map_or(0, |step| step.count).max(1) as f64;
	let mut row = View()
		.class("flex-row items-end gap-2")
		.role(Role::Img)
		.aria_label(
			steps
				.iter()
				.enumerate()
				.map(|(index, step)| {
					format!(
						"{}. {}: {}",
						index + 1,
						step.name,
						format::thousands(step.count)
					)
				})
				.collect::<Vec<_>>()
				.join(", "),
		)
		.test_id("funnel-chart");
	let mut previous: Option<u64> = None;

	for (index, step) in steps.iter().enumerate() {
		if let Some(conversion) = step.conversion {
			row = row.child(
				View()
					.class("w-16 shrink-0 flex-col items-center gap-0.5 pb-3 max-sm:w-10")
					.child(
						Text(format!("↘ {}", format::share(conversion)))
							.class("text-xs font-semibold tabular-nums text-[var(--admin-ink)]"),
					),
			);
		}

		let filled = step.count as f64 / first;
		let lost = previous.map_or(0.0, |previous| {
			previous.saturating_sub(step.count) as f64 / first
		});
		let mut column = View()
			.class("relative w-full flex-col justify-end")
			.class(height);

		if lost > 0.0 {
			let hatched = View()
				.class("relative w-full overflow-hidden rounded-[3px]")
				.class(format!("h-[{}%]", percent(lost)))
				.child(hatch());
			column = column.child(match &step.dropped {
				Some(href) => {
					Pressable(format!(
						"Show {} who dropped off before {}",
						people(previous.unwrap_or(0).saturating_sub(step.count)),
						step.name
					))
					.href(href.clone())
					.class("w-full cursor-pointer")
					.class(format!("h-[{}%]", percent(lost)))
					.test_id("funnel-drop-off")
					.child(hatched.class("h-full"))
				}
				None => hatched,
			});
		}

		let bar = View()
			.class("w-full rounded-[3px] bg-[var(--admin-series-1)]")
			.class(format!("h-[{}%]", percent(filled.max(0.004))));
		column = column.child(match &step.reached {
			Some(href) => {
				Pressable(format!(
					"Show {} who reached {}",
					people(step.count),
					step.name
				))
				.href(href.clone())
				.class("w-full cursor-pointer")
				.class(format!("h-[{}%]", percent(filled.max(0.004))))
				.test_id("funnel-step")
				.child(bar.class("h-full"))
			}
			None => bar,
		});

		row = row.child(
			View()
				.class("min-w-0 flex-1 flex-col items-center gap-1.5")
				.child(
					Text(format::thousands(step.count))
						.class("text-sm font-semibold tabular-nums text-[var(--admin-ink)]"),
				)
				.child(column)
				.child(Text(format!("{}. {}", index + 1, step.name)).class(
					"w-full truncate text-center text-xs font-semibold text-[var(--admin-ink-2)]",
				)),
		);
		previous = Some(step.count);
	}

	row
}

/// `the 1 person`, `the 1,204 people`.
fn people(count: u64) -> String {
	if count == 1 {
		"the 1 person".to_owned()
	} else {
		format!("the {} people", format::thousands(count))
	}
}

/// The sequential ramp's step for a share, 1 to 6.
fn ramp(share: f64) -> usize {
	match share {
		share if share >= 0.6 => 6,
		share if share >= 0.45 => 5,
		share if share >= 0.3 => 4,
		share if share >= 0.18 => 3,
		share if share >= 0.08 => 2,
		_ => 1,
	}
}

fn cell_class(step: usize) -> &'static str {
	match step {
		1 => "bg-[var(--admin-seq-1)] text-[var(--admin-seq-text-1)]",
		2 => "bg-[var(--admin-seq-2)] text-[var(--admin-seq-text-2)]",
		3 => "bg-[var(--admin-seq-3)] text-[var(--admin-seq-text-3)]",
		4 => "bg-[var(--admin-seq-4)] text-[var(--admin-seq-text-4)]",
		5 => "bg-[var(--admin-seq-5)] text-[var(--admin-seq-text-5)]",
		6 => "bg-[var(--admin-seq-6)] text-[var(--admin-seq-text-6)]",
		_ => "bg-[var(--admin-sunken)] text-[var(--admin-ink)]",
	}
}

/// One cohort row.
pub(crate) struct Cohort {
	pub label: String,
	pub size: u64,
	/// Each period's returns, from period 0.
	pub returns: Vec<u64>,
	/// Where pressing each cell goes.
	pub targets: Vec<Option<String>>,
}

/// The retention grid: a bold mean row on top, a row per cohort, a cell per
/// period in the sequential ramp, period 0 in `--admin-sunken`.
#[expect(
	clippy::cast_precision_loss,
	reason = "people counts become shares for drawing"
)]
pub(crate) fn retention(cohorts: &[Cohort], period: &str, mean: &[f64]) -> Element {
	let periods = cohorts
		.iter()
		.map(|cohort| cohort.returns.len())
		.max()
		.unwrap_or(0);
	let cell = |text: String, class: &str| {
		Text(text)
			.class(
				"h-9 w-14 shrink-0 items-center justify-center rounded-[3px] text-center \
				 text-[13px] leading-9 tabular-nums",
			)
			.class(class)
	};
	let mut header = View()
		.class("flex-row gap-1")
		.child(Text("Cohort").class("w-20 shrink-0").class(style::LABEL))
		.child(Text("People").class("w-16 shrink-0").class(style::LABEL));

	for index in 0..periods {
		header = header.child(
			Text(format!("{period}{index}"))
				.class("w-14 shrink-0 text-center")
				.class(style::LABEL),
		);
	}

	let total: u64 = cohorts.iter().map(|cohort| cohort.size).sum();
	let mut mean_row = View()
		.class("flex-row items-center gap-1")
		.test_id("retention-mean")
		.child(Text("Mean").class("w-20 shrink-0 text-sm font-semibold text-[var(--admin-ink)]"))
		.child(
			Text(format::thousands(total))
				.class("w-16 shrink-0 text-sm font-semibold tabular-nums text-[var(--admin-ink)]"),
		);

	for (index, share) in mean.iter().enumerate() {
		let class = if index == 0 {
			cell_class(0)
		} else {
			cell_class(ramp(*share))
		};
		mean_row =
			mean_row.child(cell(format!("{:.0}%", share * 100.0), class).class("font-semibold"));
	}

	let mut grid = View()
		.class("flex-col gap-1")
		.role(Role::Table)
		.aria_label("Retention by cohort")
		.child(header)
		.child(mean_row);

	for cohort in cohorts {
		let mut row = View()
			.class("flex-row items-center gap-1")
			.child(
				Text(cohort.label.clone()).class("w-20 shrink-0 text-sm text-[var(--admin-ink)]"),
			)
			.child(
				Text(format::thousands(cohort.size))
					.class("w-16 shrink-0 text-sm tabular-nums text-[var(--admin-ink-2)]"),
			);

		for (index, returned) in cohort.returns.iter().enumerate() {
			let share = if cohort.size > 0 {
				*returned as f64 / cohort.size as f64
			} else {
				0.0
			};
			let class = if index == 0 {
				cell_class(0)
			} else {
				cell_class(ramp(share))
			};
			let text = cell(format!("{:.0}%", share * 100.0), class);
			row = row.child(match cohort.targets.get(index).cloned().flatten() {
				Some(href) => {
					Pressable(format!(
						"Show {} from the {} cohort who came back in {}{index}",
						people(*returned),
						cohort.label,
						period.to_lowercase()
					))
					.href(href)
					.class("cursor-pointer rounded-[3px] active:opacity-80")
					.test_id("retention-cell")
					.child(text)
				}
				None => text,
			});
		}

		grid = grid.child(row);
	}

	View().class("overflow-scroll pb-1").child(grid)
}

/// A sparkline: a 1.5px line in series 1, the last point as a dot.
pub(crate) fn sparkline(values: &[f64], class: &str) -> Element {
	let view_box = ViewBox::new(0.0, 0.0, 100.0, 28.0).expect("a valid view box");
	let mut line = Vector(view_box)
		.scaling(VectorScaling::Stretch)
		.class("absolute inset-0 text-[var(--admin-series-1)]")
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
					index as f32 / (values.len() - 1) as f32 * 97.0
				},
				y: 25.0 - (value / highest) as f32 * 22.0,
			}
		})
		.collect();

	if points.len() > 1
		&& let Ok(data) = PathData::polyline(points.clone())
	{
		line = line.path(VectorPath::new(data).stroke(stroke(1.5)));
	}

	let mut spark = View()
		.class("relative")
		.class(class)
		.aria_hidden(true)
		.child(line);

	if let Some(last) = points.last() {
		spark = spark.child(
			View()
				.class(
					"absolute h-1.5 w-1.5 -ml-[3px] -mt-[3px] rounded-full \
					 bg-[var(--admin-series-1)]",
				)
				.class(format!(
					"left-[{}%] top-[{}%]",
					(f64::from(last.x) * 10.0).round() / 10.0,
					(f64::from(last.y) / 28.0 * 1000.0).round() / 10.0
				)),
		);
	}

	spark
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ceilings_quarter_tidily() {
		assert!((ceiling(1650.0) - 2000.0).abs() < f64::EPSILON);
		assert!((ceiling(180.0) - 200.0).abs() < f64::EPSILON);
		assert!((ceiling(0.0) - 4.0).abs() < f64::EPSILON);
		assert!((ceiling(230.0) - 300.0).abs() < f64::EPSILON);
		assert!((ceiling(0.41) - 0.6).abs() < 1e-9);
	}
}
