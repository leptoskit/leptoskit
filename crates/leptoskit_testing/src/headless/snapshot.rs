//! Deterministic pictures of a laid-out tree, without a GPU or a window.
//!
//! The tree is first flattened into a display list of rectangles, outlines,
//! circles, lines, and text runs, then written as SVG or rasterised to PNG
//! with tiny-skia. SVG keeps text as `<text>`; PNG has no fonts, so each line
//! of text becomes a bar as wide as layout measured it. Native controls draw
//! as simple outlines, since a platform would draw their chrome.

use std::fmt::Write;
use std::path::Path;

use leptoskit_native::Color;
use leptoskit_native::FontWeight;
use leptoskit_native::Frame;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::Progress;
use leptoskit_native::TextAlign;
use leptoskit_native::TextStyle;
use leptoskit_yoga::NodeId;

use super::HeadlessError;
use crate::inspect::Inspector;
use crate::inspect::clips;
use crate::inspect::hides;

/// The largest PNG a snapshot renders, in pixels.
const MAX_PIXELS: f32 = 32_000_000.0;
/// The approximate measurer's cell at [`TextStyle::DEFAULT_FONT_SIZE`].
const CELL_WIDTH: f32 = 8.0;
const LINE_HEIGHT: f32 = 16.0;
/// Inset of a text input's text from its edges.
const INPUT_INSET: f32 = 8.0;

const OUTLINE: Color = Color::rgba(0x8e, 0x8e, 0x93, 0xff);
const CONTROL: Color = Color::rgba(0xe5, 0xe5, 0xea, 0xff);
const ACCENT: Color = Color::rgba(0x00, 0x7a, 0xff, 0xff);
const SWITCH_ON: Color = Color::rgba(0x34, 0xc7, 0x59, 0xff);
const KNOB: Color = Color::rgba(0xff, 0xff, 0xff, 0xff);
/// Tailwind's preflight border color, for a border with no color of its own.
const BORDER: Color = Color::rgba(0xe5, 0xe7, 0xeb, 0xff);

/// What a snapshot is written as.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapshotFormat {
	/// SVG text, with real `<text>` elements.
	Svg,
	/// A PNG at `scale` pixels per layout unit, with text drawn as bars.
	Png {
		/// Pixels per layout unit.
		scale: f32,
	},
}

impl SnapshotFormat {
	/// The format a file extension names: `.svg` or `.png`.
	pub fn for_path(path: &Path, scale: f32) -> Result<Self, String> {
		match path.extension().and_then(|extension| extension.to_str()) {
			Some("svg") => Ok(Self::Svg),
			Some("png") => Ok(Self::Png { scale }),
			_ => Err(format!("{} must end in .svg or .png", path.display())),
		}
	}
}

/// A rendered snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
	bytes: Vec<u8>,
	mime_type: &'static str,
}

impl Snapshot {
	/// The encoded file.
	pub fn bytes(&self) -> &[u8] {
		&self.bytes
	}

	/// `image/svg+xml` or `image/png`.
	pub fn mime_type(&self) -> &'static str {
		self.mime_type
	}

	/// Writes the file.
	pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
		std::fs::write(path, &self.bytes)
	}
}

/// One drawing operation, in viewport coordinates.
#[derive(Debug, Clone, PartialEq)]
enum Paint {
	Fill {
		rect: Frame,
		radius: f32,
		color: Color,
	},
	Stroke {
		rect: Frame,
		radius: f32,
		width: f32,
		color: Color,
	},
	Circle {
		x: f32,
		y: f32,
		radius: f32,
		color: Color,
	},
	Line {
		from: (f32, f32),
		to: (f32, f32),
		width: f32,
		color: Color,
	},
	Text {
		x: f32,
		baseline: f32,
		width: f32,
		size: f32,
		weight: u16,
		italic: bool,
		color: Color,
		content: String,
	},
	/// Clips what follows to a rounded rectangle, until the matching pop.
	Clip {
		rect: Frame,
		radius: f32,
	},
	/// Fades what follows, until the matching pop.
	Opacity(f32),
	Pop,
}

pub(super) fn render<B: NativeBackend>(
	inspect: &Inspector<'_, B>,
	dark: bool,
	format: SnapshotFormat,
) -> Result<Snapshot, HeadlessError> {
	let mut paints = Vec::new();
	let painter = Painter {
		inspect,
		text: if dark {
			Color::rgba(0xff, 0xff, 0xff, 0xff)
		} else {
			Color::rgba(0, 0, 0, 0xff)
		},
	};

	if let Some(root) = inspect.surface.root() {
		painter.node(root, &mut paints);
	}

	let background = if dark {
		Color::rgba(0, 0, 0, 0xff)
	} else {
		Color::rgba(0xff, 0xff, 0xff, 0xff)
	};
	let (width, height) = inspect.viewport;

	match format {
		SnapshotFormat::Svg => {
			Ok(Snapshot {
				bytes: svg(width, height, background, &paints).into_bytes(),
				mime_type: "image/svg+xml",
			})
		}
		SnapshotFormat::Png { scale } => {
			Ok(Snapshot {
				bytes: png(width, height, scale, background, &paints)?,
				mime_type: "image/png",
			})
		}
	}
}

struct Painter<'a, 'b, B: NativeBackend> {
	inspect: &'a Inspector<'b, B>,
	/// The default text color for the appearance.
	text: Color,
}

impl<B: NativeBackend> Painter<'_, '_, B> {
	fn node(&self, id: NodeId, paints: &mut Vec<Paint>) {
		let surface = self.inspect.surface;
		let kind = surface.kind(id);
		let props = surface.props(id);
		let frame = self.inspect.absolute_frame(id);

		if hides(props, kind) || frame.width <= 0.0 || frame.height <= 0.0 {
			return;
		}

		let radius = props
			.corner_radius
			.unwrap_or(0.0)
			.min(frame.width / 2.0)
			.min(frame.height / 2.0)
			.max(0.0);
		let mut pushed = 0;

		if let Some(opacity) = props.opacity.filter(|opacity| *opacity < 1.0) {
			paints.push(Paint::Opacity(opacity));
			pushed += 1;
		}

		if let Some(color) = props.background {
			paints.push(Paint::Fill {
				rect: frame,
				radius,
				color,
			});
		}

		Self::chrome(kind, props, frame, paints);
		Self::border(props, frame, radius, paints);
		self.content(id, kind, props, frame, paints);

		if clips(props, kind) {
			paints.push(Paint::Clip {
				rect: frame,
				radius,
			});
			pushed += 1;
		}

		// A text's runs draw with it.
		if kind != NativeKind::Text {
			for &child in surface.children(id) {
				self.node(child, paints);
			}
		}

		paints.extend(std::iter::repeat_n(Paint::Pop, pushed));
	}

	fn border(props: &NativeProps, frame: Frame, radius: f32, paints: &mut Vec<Paint>) {
		let color = props.border_color.unwrap_or(BORDER);

		if let Some(widths) = props.border_widths {
			let Frame {
				x,
				y,
				width,
				height,
			} = frame;

			for rect in [
				(x, y, width, widths.top),
				(x + width - widths.right, y, widths.right, height),
				(x, y + height - widths.bottom, width, widths.bottom),
				(x, y, widths.left, height),
			] {
				if rect.2 > 0.0 && rect.3 > 0.0 {
					paints.push(Paint::Fill {
						rect: Frame {
							x: rect.0,
							y: rect.1,
							width: rect.2,
							height: rect.3,
						},
						radius: 0.0,
						color,
					});
				}
			}
		} else if let Some(width) = props.border_width.filter(|width| *width > 0.0) {
			paints.push(Paint::Stroke {
				rect: inset(frame, width / 2.0),
				radius: (radius - width / 2.0).max(0.0),
				width,
				color,
			});
		}
	}

	/// What a platform would draw for a native control.
	fn chrome(kind: NativeKind, props: &NativeProps, frame: Frame, paints: &mut Vec<Paint>) {
		let styled = props.background.is_some() || props.border_width.is_some();
		let Frame {
			x,
			y,
			width,
			height,
		} = frame;
		let (center_x, center_y) = (x + width / 2.0, y + height / 2.0);

		match kind {
			NativeKind::Button | NativeKind::TextInput | NativeKind::PasswordInput if !styled => {
				paints.push(Paint::Stroke {
					rect: inset(frame, 0.5),
					radius: 6.0_f32.min(height / 2.0),
					width: 1.0,
					color: OUTLINE,
				});
			}
			NativeKind::Switch => {
				let on = props.checked.unwrap_or(false);
				paints.push(Paint::Fill {
					rect: frame,
					radius: height / 2.0,
					color: if on { SWITCH_ON } else { CONTROL },
				});
				let knob = (height / 2.0 - 2.0).max(0.0);
				paints.push(Paint::Circle {
					x: if on {
						x + width - height / 2.0
					} else {
						x + height / 2.0
					},
					y: center_y,
					radius: knob,
					color: KNOB,
				});
			}
			NativeKind::Checkbox | NativeKind::Radio => {
				let size = width.min(height);
				let box_frame = Frame {
					x,
					y: center_y - size / 2.0,
					width: size,
					height: size,
				};
				let round = if kind == NativeKind::Radio {
					size / 2.0
				} else {
					size / 5.0
				};

				if props.checked.unwrap_or(false) {
					paints.push(Paint::Fill {
						rect: box_frame,
						radius: round,
						color: ACCENT,
					});
				} else {
					paints.push(Paint::Stroke {
						rect: inset(box_frame, 0.5),
						radius: round,
						width: 1.0,
						color: OUTLINE,
					});
				}
			}
			NativeKind::Slider => {
				let slider = props.slider.unwrap_or_default();
				let span = slider.max() - slider.min();
				let fraction = if span > 0.0 {
					((slider.value() - slider.min()) / span).clamp(0.0, 1.0) as f32
				} else {
					0.0
				};
				let knob = (height / 2.0).min(10.0);
				let track = (x + knob, x + width - knob);
				let at = track.0 + (track.1 - track.0) * fraction;
				paints.push(Paint::Line {
					from: (track.0, center_y),
					to: (track.1, center_y),
					width: 2.0,
					color: CONTROL,
				});
				paints.push(Paint::Line {
					from: (track.0, center_y),
					to: (at, center_y),
					width: 2.0,
					color: ACCENT,
				});
				paints.push(Paint::Circle {
					x: at,
					y: center_y,
					radius: knob,
					color: OUTLINE,
				});
			}
			NativeKind::LinearProgressIndicator => {
				paints.push(Paint::Fill {
					rect: frame,
					radius: height / 2.0,
					color: CONTROL,
				});

				if let Some(Progress::Determinate(value)) = props.progress {
					paints.push(Paint::Fill {
						rect: Frame {
							width: width * value.get() as f32,
							..frame
						},
						radius: height / 2.0,
						color: ACCENT,
					});
				}
			}
			NativeKind::ActivityIndicator => {
				let radius = width.min(height) / 2.0;
				paints.push(Paint::Stroke {
					rect: Frame {
						x: center_x - radius + 1.0,
						y: center_y - radius + 1.0,
						width: radius * 2.0 - 2.0,
						height: radius * 2.0 - 2.0,
					},
					radius: radius - 1.0,
					width: 2.0,
					color: OUTLINE,
				});
			}
			NativeKind::Image | NativeKind::Vector | NativeKind::Rive => {
				if props.background.is_none() {
					paints.push(Paint::Fill {
						rect: frame,
						radius: props.corner_radius.unwrap_or(0.0),
						color: CONTROL,
					});
				}

				paints.push(Paint::Line {
					from: (x, y),
					to: (x + width, y + height),
					width: 1.0,
					color: OUTLINE,
				});
				paints.push(Paint::Line {
					from: (x + width, y),
					to: (x, y + height),
					width: 1.0,
					color: OUTLINE,
				});
			}
			_ => {}
		}
	}

	/// The text a widget shows.
	fn content(
		&self,
		id: NodeId,
		kind: NativeKind,
		props: &NativeProps,
		frame: Frame,
		paints: &mut Vec<Paint>,
	) {
		let surface = self.inspect.surface;
		let (content, color, inset_x, centered) = match kind {
			NativeKind::Text => {
				let content = props
					.text
					.clone()
					.or_else(|| {
						props
							.spans
							.as_ref()
							.map(|spans| spans.iter().map(|span| span.text.as_str()).collect())
					})
					.unwrap_or_else(|| {
						surface
							.children(id)
							.iter()
							.filter_map(|&child| surface.props(child).text.as_deref())
							.collect()
					});
				(content, props.color.unwrap_or(self.text), 0.0, false)
			}
			NativeKind::Button => {
				let color = props.color.unwrap_or(if props.background.is_some() {
					self.text
				} else {
					ACCENT
				});
				(props.text.clone().unwrap_or_default(), color, 0.0, true)
			}
			NativeKind::TextInput | NativeKind::PasswordInput => {
				match props.value.as_deref().filter(|value| !value.is_empty()) {
					Some(value) if kind == NativeKind::PasswordInput => {
						(
							"•".repeat(value.chars().count()),
							props.color.unwrap_or(self.text),
							INPUT_INSET,
							false,
						)
					}
					Some(value) => {
						(
							value.to_owned(),
							props.color.unwrap_or(self.text),
							INPUT_INSET,
							false,
						)
					}
					None => {
						(
							props.placeholder.clone().unwrap_or_default(),
							OUTLINE,
							INPUT_INSET,
							false,
						)
					}
				}
			}
			_ => return,
		};

		if content.is_empty() {
			return;
		}

		let size = props
			.font_size
			.filter(|size| size.is_finite() && *size > 0.0)
			.unwrap_or(TextStyle::DEFAULT_FONT_SIZE);
		let scale = size / TextStyle::DEFAULT_FONT_SIZE;
		let (cell, line_height) = (CELL_WIDTH * scale, LINE_HEIGHT * scale);
		let available = (frame.width - inset_x * 2.0).max(0.0);
		let lines = wrap(&content, cell, available);
		let block = line_height * lines.len() as f32;
		// Inputs and buttons centre their text vertically, as their controls
		// do.
		let top = if centered || inset_x > 0.0 {
			frame.y + (frame.height - block) / 2.0
		} else {
			frame.y
		};
		let align = if centered {
			TextAlign::Center
		} else {
			props.text_align.unwrap_or_default()
		};

		for (index, (line, width)) in lines.into_iter().enumerate() {
			let x = match align {
				TextAlign::Center => frame.x + (frame.width - width) / 2.0,
				TextAlign::End => frame.x + frame.width - inset_x - width,
				TextAlign::Start => frame.x + inset_x,
			};
			paints.push(Paint::Text {
				x,
				baseline: top + line_height * index as f32 + line_height * 0.75,
				width,
				size,
				weight: weight(props.font_weight.unwrap_or_default()),
				italic: props.italic.unwrap_or(false),
				color,
				content: line,
			});
		}
	}
}

/// Breaks text into lines the way the approximate measurer does, with each
/// line's measured width.
fn wrap(text: &str, cell: f32, max: f32) -> Vec<(String, f32)> {
	let mut lines = Vec::new();

	for paragraph in text.split('\n') {
		let mut line = String::new();
		let mut width = 0.0f32;

		for word in paragraph.split(' ') {
			// The measurer clamps a word wider than the line to the line.
			let word_width = (cell * word.chars().count() as f32).min(max);
			let space = if width > 0.0 { cell } else { 0.0 };

			if width > 0.0 && width + space + word_width > max {
				lines.push((std::mem::take(&mut line), width));
				width = word_width;
			} else {
				if width > 0.0 {
					line.push(' ');
				}

				width += space + word_width;
			}

			line.push_str(word);
		}

		lines.push((line, width));
	}

	lines
}

fn weight(weight: FontWeight) -> u16 {
	match weight {
		FontWeight::Regular => 400,
		FontWeight::Medium => 500,
		FontWeight::Semibold => 600,
		FontWeight::Bold => 700,
	}
}

fn inset(frame: Frame, by: f32) -> Frame {
	Frame {
		x: frame.x + by,
		y: frame.y + by,
		width: (frame.width - by * 2.0).max(0.0),
		height: (frame.height - by * 2.0).max(0.0),
	}
}

/// A coordinate rounded to a hundredth, without float noise.
fn num(value: f32) -> String {
	let rounded = (value * 100.0).round() / 100.0;

	if rounded == 0.0 {
		"0".to_owned()
	} else {
		rounded.to_string()
	}
}

fn hex(color: Color) -> String {
	format!(
		"#{:02x}{:02x}{:02x}",
		color.red(),
		color.green(),
		color.blue()
	)
}

/// ` fill="#rrggbb"` plus its opacity when it is not opaque.
fn svg_color(attribute: &str, color: Color) -> String {
	let mut output = format!(" {attribute}=\"{}\"", hex(color));

	if color.alpha() < 0xff {
		write!(
			output,
			" {attribute}-opacity=\"{}\"",
			num(f32::from(color.alpha()) / 255.0)
		)
		.expect("writing to a string");
	}

	output
}

fn svg_rect(frame: Frame, radius: f32) -> String {
	let mut output = format!(
		"x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
		num(frame.x),
		num(frame.y),
		num(frame.width),
		num(frame.height)
	);

	if radius > 0.0 {
		write!(output, " rx=\"{}\"", num(radius)).expect("writing to a string");
	}

	output
}

fn escape(text: &str) -> String {
	let mut output = String::with_capacity(text.len());

	for character in text.chars() {
		match character {
			'&' => output.push_str("&amp;"),
			'<' => output.push_str("&lt;"),
			'>' => output.push_str("&gt;"),
			'"' => output.push_str("&quot;"),
			// XML 1.0 has no representation for most control characters.
			character if character.is_control() => output.push(' '),
			character => output.push(character),
		}
	}

	output
}

fn svg(width: f32, height: f32, background: Color, paints: &[Paint]) -> String {
	let (width, height) = (num(width), num(height));
	let mut output = format!(
		"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
		 viewBox=\"0 0 {width} {height}\" font-family=\"monospace\">\n<rect width=\"{width}\" \
		 height=\"{height}\"{}/>\n",
		svg_color("fill", background)
	);
	let mut depth = 0usize;
	let mut clips = 0usize;

	for paint in paints {
		let indent = "  ".repeat(depth);
		// Writing to a String cannot fail.
		match paint {
			Paint::Fill {
				rect,
				radius,
				color,
			} => {
				writeln!(
					output,
					"{indent}<rect {}{}/>",
					svg_rect(*rect, *radius),
					svg_color("fill", *color)
				)
			}
			Paint::Stroke {
				rect,
				radius,
				width,
				color,
			} => {
				writeln!(
					output,
					"{indent}<rect {} fill=\"none\"{} stroke-width=\"{}\"/>",
					svg_rect(*rect, *radius),
					svg_color("stroke", *color),
					num(*width)
				)
			}
			Paint::Circle {
				x,
				y,
				radius,
				color,
			} => {
				writeln!(
					output,
					"{indent}<circle cx=\"{}\" cy=\"{}\" r=\"{}\"{}/>",
					num(*x),
					num(*y),
					num(*radius),
					svg_color("fill", *color)
				)
			}
			Paint::Line {
				from,
				to,
				width,
				color,
			} => {
				writeln!(
					output,
					"{indent}<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"{} stroke-width=\"{}\"/>",
					num(from.0),
					num(from.1),
					num(to.0),
					num(to.1),
					svg_color("stroke", *color),
					num(*width)
				)
			}
			Paint::Text {
				x,
				baseline,
				width,
				size,
				weight,
				italic,
				color,
				content,
			} => {
				let mut attributes = format!(
					"x=\"{}\" y=\"{}\" font-size=\"{}\"",
					num(*x),
					num(*baseline),
					num(*size)
				);

				if *weight != 400 {
					write!(attributes, " font-weight=\"{weight}\"").expect("writing to a string");
				}

				if *italic {
					attributes.push_str(" font-style=\"italic\"");
				}

				if *width > 0.0 {
					write!(
						attributes,
						" textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\"",
						num(*width)
					)
					.expect("writing to a string");
				}

				writeln!(
					output,
					"{indent}<text {attributes}{} xml:space=\"preserve\">{}</text>",
					svg_color("fill", *color),
					escape(content)
				)
			}
			Paint::Clip { rect, radius } => {
				clips += 1;
				depth += 1;
				writeln!(
					output,
					"{indent}<clipPath id=\"clip{clips}\"><rect {}/></clipPath>\n{indent}<g \
					 clip-path=\"url(#clip{clips})\">",
					svg_rect(*rect, *radius)
				)
			}
			Paint::Opacity(opacity) => {
				depth += 1;
				writeln!(
					output,
					"{indent}<g opacity=\"{}\">",
					num(opacity.clamp(0.0, 1.0))
				)
			}
			Paint::Pop => {
				depth = depth.saturating_sub(1);
				writeln!(output, "{}</g>", "  ".repeat(depth))
			}
		}
		.expect("writing to a string");
	}

	output.push_str("</svg>\n");
	output
}

fn png(
	width: f32,
	height: f32,
	scale: f32,
	background: Color,
	paints: &[Paint],
) -> Result<Vec<u8>, HeadlessError> {
	use tiny_skia::FillRule;
	use tiny_skia::Mask;
	use tiny_skia::Pixmap;
	use tiny_skia::Transform;

	if !scale.is_finite() || scale <= 0.0 || width * height * scale * scale > MAX_PIXELS {
		return Err(HeadlessError::Snapshot(format!(
			"a {width}x{height} snapshot at scale {scale} is outside the {MAX_PIXELS} pixel budget"
		)));
	}

	let (pixels_wide, pixels_high) = (
		(width * scale).ceil() as u32,
		(height * scale).ceil() as u32,
	);
	let mut pixmap = Pixmap::new(pixels_wide, pixels_high)
		.ok_or_else(|| HeadlessError::Snapshot("the snapshot has no pixels".to_owned()))?;
	pixmap.fill(skia_color(background, 1.0));
	let transform = Transform::from_scale(scale, scale);
	// Each entry is the clip and opacity in force inside one group.
	let mut groups: Vec<(Option<Mask>, f32)> = vec![(None, 1.0)];

	for paint in paints {
		let (mask, opacity) = groups.last().cloned().expect("the base group remains");

		match paint {
			Paint::Clip { rect, radius } => {
				let mut clip = mask.unwrap_or_else(|| {
					let mut mask = Mask::new(pixels_wide, pixels_high).expect("pixels exist");
					mask.data_mut().fill(u8::MAX);
					mask
				});

				if let Some(path) = rounded_rect(*rect, *radius) {
					clip.intersect_path(&path, FillRule::Winding, true, transform);
				} else {
					clip.data_mut().fill(0);
				}

				groups.push((Some(clip), opacity));
			}
			Paint::Opacity(alpha) => groups.push((mask, opacity * alpha.clamp(0.0, 1.0))),
			Paint::Pop => {
				if groups.len() > 1 {
					groups.pop();
				}
			}
			paint => draw(&mut pixmap, paint, transform, mask.as_ref(), opacity),
		}
	}

	pixmap
		.encode_png()
		.map_err(|error| HeadlessError::Snapshot(error.to_string()))
}

/// Draws one shape or text bar; groups are handled by [`png`].
fn draw(
	pixmap: &mut tiny_skia::Pixmap,
	paint: &Paint,
	transform: tiny_skia::Transform,
	mask: Option<&tiny_skia::Mask>,
	opacity: f32,
) {
	use tiny_skia::FillRule;
	use tiny_skia::PathBuilder;
	use tiny_skia::Stroke;

	let brush = |color: Color, alpha: f32| {
		let mut brush = tiny_skia::Paint::default();
		brush.set_color(skia_color(color, opacity * alpha));
		brush.anti_alias = true;
		brush
	};
	let stroke = |width: f32| {
		Stroke {
			width,
			..Stroke::default()
		}
	};

	match paint {
		Paint::Fill {
			rect,
			radius,
			color,
		} => {
			if let Some(path) = rounded_rect(*rect, *radius) {
				pixmap.fill_path(
					&path,
					&brush(*color, 1.0),
					FillRule::Winding,
					transform,
					mask,
				);
			}
		}
		Paint::Stroke {
			rect,
			radius,
			width,
			color,
		} => {
			if let Some(path) = rounded_rect(*rect, *radius) {
				pixmap.stroke_path(&path, &brush(*color, 1.0), &stroke(*width), transform, mask);
			}
		}
		Paint::Circle {
			x,
			y,
			radius,
			color,
		} => {
			if let Some(path) = PathBuilder::from_circle(*x, *y, *radius) {
				pixmap.fill_path(
					&path,
					&brush(*color, 1.0),
					FillRule::Winding,
					transform,
					mask,
				);
			}
		}
		Paint::Line {
			from,
			to,
			width,
			color,
		} => {
			let mut builder = PathBuilder::new();
			builder.move_to(from.0, from.1);
			builder.line_to(to.0, to.1);

			if let Some(path) = builder.finish() {
				pixmap.stroke_path(&path, &brush(*color, 1.0), &stroke(*width), transform, mask);
			}
		}
		Paint::Text {
			x,
			baseline,
			width,
			size,
			color,
			..
		} => {
			// No fonts: a bar spanning the line's x-height stands in for it.
			let bar = Frame {
				x: *x,
				y: baseline - size * 0.55,
				width: *width,
				height: size * 0.5,
			};

			if let Some(path) = rounded_rect(bar, 0.0) {
				pixmap.fill_path(
					&path,
					&brush(*color, 0.6),
					FillRule::Winding,
					transform,
					mask,
				);
			}
		}
		Paint::Clip { .. } | Paint::Opacity(_) | Paint::Pop => {}
	}
}

fn skia_color(color: Color, opacity: f32) -> tiny_skia::Color {
	tiny_skia::Color::from_rgba8(
		color.red(),
		color.green(),
		color.blue(),
		(f32::from(color.alpha()) * opacity.clamp(0.0, 1.0)).round() as u8,
	)
}

/// A rectangle with circular corners, built from cubic arcs.
fn rounded_rect(frame: Frame, radius: f32) -> Option<tiny_skia::Path> {
	let Frame {
		x,
		y,
		width,
		height,
	} = frame;

	if width <= 0.0 || height <= 0.0 {
		return None;
	}

	let radius = radius.min(width / 2.0).min(height / 2.0).max(0.0);

	if radius == 0.0 {
		let rect = tiny_skia::Rect::from_xywh(x, y, width, height)?;
		return Some(tiny_skia::PathBuilder::from_rect(rect));
	}

	// The control point distance that makes a cubic approximate a quarter
	// circle.
	let handle = radius * 0.552_284_8;
	let (right, bottom) = (x + width, y + height);
	let mut builder = tiny_skia::PathBuilder::new();
	builder.move_to(x + radius, y);
	builder.line_to(right - radius, y);
	builder.cubic_to(
		right - radius + handle,
		y,
		right,
		y + radius - handle,
		right,
		y + radius,
	);
	builder.line_to(right, bottom - radius);
	builder.cubic_to(
		right,
		bottom - radius + handle,
		right - radius + handle,
		bottom,
		right - radius,
		bottom,
	);
	builder.line_to(x + radius, bottom);
	builder.cubic_to(
		x + radius - handle,
		bottom,
		x,
		bottom - radius + handle,
		x,
		bottom - radius,
	);
	builder.line_to(x, y + radius);
	builder.cubic_to(
		x,
		y + radius - handle,
		x + radius - handle,
		y,
		x + radius,
		y,
	);
	builder.close();
	builder.finish()
}
