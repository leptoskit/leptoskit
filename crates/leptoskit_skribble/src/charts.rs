//! Hand-drawn financial charts: candlesticks, OHLC bars, lines, areas,
//! volume bars, and technical overlays.
//!
//! The drawing comes from [`skribble_core::charts`], so a seed wobbles the
//! same as skribble's Flutter and Jaspr charts. These components size the ink,
//! lay out bar labels, and describe each chart for assistive technology.
//! Charts take an explicit `width` and `height` in logical pixels, so candle
//! bodies and wicks keep the exact positions the data gives them.
//!
//! | Component                                           | Draws                                                  |
//! | --------------------------------------------------- | ------------------------------------------------------ |
//! | [`WiredCandlestickChart`](fn@WiredCandlestickChart) | Candles, OHLC bars, lines, areas, volume, and overlays |
//! | [`WiredLineChart`](fn@WiredLineChart)               | A hand-drawn line through values or points             |
//! | [`WiredBarChart`](fn@WiredBarChart)                 | Labelled bars growing from zero                        |
//!
//! [`indicators`] has `sma`, `ema`, `bollinger`, `rsi`, and `macd`, and
//! [`geometry`] maps prices and indices to pixels for callers that need the
//! same coordinates.

use leptoskit::native::Element;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit::yoga::Dimension;
use skribble_core::charts::BarChart;
pub use skribble_core::charts::Candle;
use skribble_core::charts::CandlestickChart;
pub use skribble_core::charts::ChartTheme;
use skribble_core::charts::LineChart;
pub use skribble_core::charts::LineSeries;
pub use skribble_core::charts::Overlay;
pub use skribble_core::charts::SeriesKind;
pub use skribble_core::charts::geometry;
pub use skribble_core::charts::indicators;

use crate::ink::vectors;
use crate::ink::with_class;

/// A hand-drawn price chart: candlesticks, OHLC bars, a line, or an area.
///
/// Candles are drawn in the order given. Rising candles use
/// [`ChartTheme::rise`], falling ones [`ChartTheme::fall`], and every body
/// keeps its exact open, high, low, and close positions; only the texture
/// wobbles. Volume, moving averages, and Bollinger Bands are optional.
///
/// The drawing paints with the element's text colour and follows the host's
/// light or dark appearance. Give it a `label` to make it an image for
/// assistive technology; without one it is decorative.
///
/// # Panics
///
/// Panics when `width` or `height` is not a positive, finite number, or when
/// an overlay's period is zero. These are mistakes in the layout or settings
/// rather than something to recover from.
///
/// ```
/// use leptoskit::prelude::*;
/// use leptoskit_skribble::charts::Candle;
/// use leptoskit_skribble::charts::Overlay;
/// use leptoskit_skribble::charts::WiredCandlestickChart;
///
/// let candles = vec![
/// 	Candle::new(1, 10.0, 12.0, 9.0, 11.0, 100.0),
/// 	Candle::new(2, 11.0, 13.0, 10.0, 12.0, 120.0),
/// ];
/// let chart = view! {
/// 	<WiredCandlestickChart
/// 		candles=candles
/// 		width=320.0
/// 		height=200.0
/// 		overlays=vec![Overlay::Sma(2)]
/// 		show_volume=true
/// 		label="Price in US dollars"
/// 	/>
/// };
/// assert!(chart.validate().is_ok());
/// ```
#[component]
pub fn WiredCandlestickChart(
	/// The intervals to draw, usually oldest first.
	candles: Vec<Candle>,
	/// Width in logical pixels.
	width: f32,
	/// Height in logical pixels.
	height: f32,
	/// Candles, OHLC bars, a line, or an area.
	#[prop(optional)]
	kind: SeriesKind,
	/// Technical overlays drawn over the price series.
	#[prop(optional)]
	overlays: Vec<Overlay>,
	/// Draws volume bars below the price pane.
	#[prop(optional)]
	show_volume: bool,
	/// Chooses the wobble. The same seed always draws the same chart.
	#[prop(default = 1)]
	seed: i64,
	/// The colours and line weight to draw with.
	#[prop(default = ChartTheme::SKRIBBLE)]
	theme: ChartTheme,
	/// Names the chart for assistive technology. Without one the chart is
	/// decorative.
	#[prop(optional, into)]
	label: Option<String>,
	/// Extra Tailwind classes.
	#[prop(optional, into)]
	class: Option<String>,
) -> Element {
	assert!(
		width.is_finite() && width > 0.0 && height.is_finite() && height > 0.0,
		"a chart needs a positive, finite size, found {width} by {height}"
	);

	let drawing = CandlestickChart {
		candles,
		kind,
		overlays,
		show_volume,
		seed,
		theme,
	}
	.drawing(width, height)
	.unwrap_or_else(|error| panic!("{error}"));
	let chart = View().children(vectors(&drawing));
	let chart = sized(chart, width, height);
	let chart = accessible(chart, label);
	with_class(chart, class)
}

/// A hand-drawn line chart over values or explicit points.
///
/// A [`LineSeries::Values`] series plots one value per evenly spaced slot; a
/// [`LineSeries::Points`] series plots each point at its own coordinates.
/// Non-finite values are skipped, leaving a gap in the line. Give it a `label`
/// to make it an image for assistive technology; without one it is
/// decorative.
///
/// # Panics
///
/// Panics when `width` or `height` is not a positive, finite number.
///
/// ```
/// use leptoskit::prelude::*;
/// use leptoskit_skribble::charts::WiredLineChart;
///
/// let chart = view! {
/// 	<WiredLineChart series=vec![1.0, 3.0, 2.0, 5.0] width=240.0 height=120.0 label="Revenue" />
/// };
/// assert!(chart.validate().is_ok());
/// ```
#[component]
pub fn WiredLineChart(
	/// The values or points to draw.
	#[prop(into)]
	series: LineSeries,
	/// Width in logical pixels.
	width: f32,
	/// Height in logical pixels.
	height: f32,
	/// Chooses the wobble. The same seed always draws the same chart.
	#[prop(default = 1)]
	seed: i64,
	/// The colours and line weight to draw with.
	#[prop(default = ChartTheme::SKRIBBLE)]
	theme: ChartTheme,
	/// Names the chart for assistive technology. Without one the chart is
	/// decorative.
	#[prop(optional, into)]
	label: Option<String>,
	/// Extra Tailwind classes.
	#[prop(optional, into)]
	class: Option<String>,
) -> Element {
	assert!(
		width.is_finite() && width > 0.0 && height.is_finite() && height > 0.0,
		"a chart needs a positive, finite size, found {width} by {height}"
	);

	let drawing = LineChart {
		series,
		seed,
		theme,
	}
	.drawing(width, height)
	.unwrap_or_else(|error| panic!("{error}"));
	let chart = View().children(vectors(&drawing));
	let chart = sized(chart, width, height);
	let chart = accessible(chart, label);
	with_class(chart, class)
}

/// A hand-drawn bar chart with a label under every bar.
///
/// Bars grow from zero; positive values use [`ChartTheme::rise`] and negative
/// ones [`ChartTheme::fall`]. The labels sit in a row below the plot, so the
/// component is `height` plus one text line tall. Give it a `label` to make it
/// an image for assistive technology; without one it is decorative.
///
/// # Panics
///
/// Panics when `width` or `height` is not a positive, finite number.
///
/// ```
/// use leptoskit::prelude::*;
/// use leptoskit_skribble::charts::WiredBarChart;
///
/// let chart = view! {
/// 	<WiredBarChart
/// 		bars=vec![("Mon".to_owned(), 4.0), ("Tue".to_owned(), -1.0)]
/// 		width=240.0
/// 		height=140.0
/// 		label="Weekday change"
/// 	/>
/// };
/// assert!(chart.validate().is_ok());
/// ```
#[component]
pub fn WiredBarChart(
	/// Bars as `(label, value)` pairs, drawn left to right.
	bars: Vec<(String, f64)>,
	/// Width in logical pixels.
	width: f32,
	/// Height in logical pixels, before the label row.
	height: f32,
	/// Chooses the wobble. The same seed always draws the same chart.
	#[prop(default = 1)]
	seed: i64,
	/// The colours and line weight to draw with.
	#[prop(default = ChartTheme::SKRIBBLE)]
	theme: ChartTheme,
	/// Names the chart for assistive technology. Without one the chart is
	/// decorative.
	#[prop(optional, into)]
	label: Option<String>,
	/// Extra Tailwind classes.
	#[prop(optional, into)]
	class: Option<String>,
) -> Element {
	assert!(
		width.is_finite() && width > 0.0 && height.is_finite() && height > 0.0,
		"a chart needs a positive, finite size, found {width} by {height}"
	);

	let chart = BarChart { bars, seed, theme };
	let drawing = chart
		.drawing(width, height)
		.unwrap_or_else(|error| panic!("{error}"));
	let plot_view = sized(View().children(vectors(&drawing)), width, height);
	let labels = View()
		.class("flex-row w-full")
		.children(chart.labels().map(|label| {
			Text(label.to_owned()).class("flex-1 text-center text-xs overflow-hidden")
		}))
		.with_style(|style| style.width = Dimension::Points(width));
	let chart_view = View()
		.class("flex-col")
		.child(plot_view)
		.child(labels)
		.with_style(|style| style.width = Dimension::Points(width));
	let chart_view = accessible(chart_view, label);
	with_class(chart_view, class)
}

fn sized(element: Element, width: f32, height: f32) -> Element {
	element.with_style(|style| {
		style.width = Dimension::Points(width);
		style.height = Dimension::Points(height);
	})
}

fn accessible(element: Element, label: Option<String>) -> Element {
	match label {
		Some(label) => element.role(Role::Img).aria_label(label),
		None => element.aria_hidden(true),
	}
}
