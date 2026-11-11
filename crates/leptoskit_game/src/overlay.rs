//! Commits a leptoskit tree and reads the boxes and labels the host should
//! draw.
//!
//! The tree is the same one every other backend receives: `NativeSurface`
//! diffs it into a mutation plan and lays it out. This module does not compile
//! a second language. It records the committed frames.

use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_yoga::NodeId;

use crate::Error;
use crate::raster::GlyphRun;
use crate::raster::ScreenRect;

/// Boxes and labels from one committed tree.
#[derive(Debug, Default)]
pub(crate) struct Overlay {
	pub rects: Vec<ScreenRect>,
	pub glyphs: Vec<GlyphRun>,
}

pub(crate) fn read(surface: &NativeSurface<HeadlessBackend>) -> Overlay {
	let mut overlay = Overlay::default();
	if let Some(root) = surface.root() {
		walk(surface, root, 0.0, 0.0, &mut overlay);
	}
	overlay
}

fn walk(
	surface: &NativeSurface<HeadlessBackend>,
	id: NodeId,
	origin_x: f32,
	origin_y: f32,
	overlay: &mut Overlay,
) {
	let Some(frame) = surface.frame(id) else {
		return;
	};
	let x = origin_x + frame.x;
	let y = origin_y + frame.y;
	let props = surface.props(id);
	if let Some(color) = props.background.filter(|color| color.alpha() > 0) {
		overlay.rects.push(ScreenRect {
			x,
			y,
			width: frame.width,
			height: frame.height,
			color: [color.red(), color.green(), color.blue(), color.alpha()],
		});
	}
	if let Some(text) = props.text.as_ref().filter(|text| !text.is_empty()) {
		let color = props
			.color
			.unwrap_or(leptoskit_native::Color::rgba(255, 255, 255, 255));
		let scale = 2.0;
		let height = 7.0 * scale;
		overlay.glyphs.push(GlyphRun {
			x: x + 8.0,
			y: y + (frame.height - height).max(0.0) / 2.0,
			text: text.clone(),
			color: [color.red(), color.green(), color.blue(), color.alpha()],
			scale,
		});
	}
	for &child in surface.children(id) {
		walk(surface, child, x, y, overlay);
	}
}

/// Presses the control whose text or accessibility label is `label`.
pub(crate) fn tap(surface: &NativeSurface<HeadlessBackend>, label: &str) -> Result<(), Error> {
	let Some(id) = find(surface, surface.root(), label) else {
		return Err(Error::Label(label.to_owned()));
	};
	let (x, y, width, height) =
		absolute(surface, id).ok_or_else(|| Error::Frame(label.to_owned()))?;
	if width <= 0.0 || height <= 0.0 {
		return Err(Error::Frame(label.to_owned()));
	}
	let hit = surface
		.hit_test(x + width / 2.0, y + height / 2.0)
		.ok_or_else(|| Error::Frame(label.to_owned()))?;
	surface.dispatch_event(hit, &NativeEvent::Press);
	Ok(())
}

fn find(
	surface: &NativeSurface<HeadlessBackend>,
	id: Option<NodeId>,
	label: &str,
) -> Option<NodeId> {
	let id = id?;
	let props = surface.props(id);
	let named =
		props.text.as_deref() == Some(label) || props.accessibility_label.as_deref() == Some(label);
	if named && surface.kind(id) == NativeKind::Button {
		return Some(id);
	}
	for &child in surface.children(id) {
		if let Some(found) = find(surface, Some(child), label) {
			return Some(found);
		}
	}
	named.then_some(id)
}

fn absolute(surface: &NativeSurface<HeadlessBackend>, id: NodeId) -> Option<(f32, f32, f32, f32)> {
	let frame = surface.frame(id)?;
	let mut x = frame.x;
	let mut y = frame.y;
	let mut current = surface.parent(id);
	while let Some(parent) = current {
		let parent_frame = surface.frame(parent)?;
		x += parent_frame.x;
		y += parent_frame.y;
		current = surface.parent(parent);
	}
	Some((x, y, frame.width, frame.height))
}

#[cfg(test)]
mod tests {
	use leptoskit_native::components::Button;
	use leptoskit_native::components::Text;
	use leptoskit_native::components::View;

	use super::*;
	use crate::Sim;
	use crate::clear_posts;
	use crate::post;

	struct Recorder;

	impl crate::Game for Recorder {
		const ID: &'static str = "recorder";

		fn setup(_: &mut crate::World) {}

		fn tick(world: &mut crate::World, input: &crate::Input) {
			if let Some(message) = input.messages().first() {
				world.publish("message", message.as_str());
			}
		}
	}

	#[test]
	fn a_menu_commits_its_bar_its_label_and_a_press() {
		clear_posts();
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_viewport(320.0, 180.0);
		surface.set_owned_root(menu());
		let overlay = read(&surface);
		assert!(overlay.glyphs.iter().any(|run| run.text == "Restart"));
		assert!(
			overlay
				.rects
				.iter()
				.any(|rect| rect.color[0] == 0 && rect.height > 0.0)
		);
		tap(&surface, "Restart").unwrap();
		let mut sim = Sim::<Recorder>::new();
		sim.step();
		assert_eq!(sim.world().text("message"), Some("restart"));
		assert!(matches!(tap(&surface, "missing"), Err(Error::Label(_))));
		surface.set_owned_root(Text("Hi").class("text-base text-white"));
		tap(&surface, "Hi").unwrap();
		surface.set_owned_root(Button("Gone").class("w-0 h-0"));
		assert!(matches!(tap(&surface, "Gone"), Err(Error::Frame(_))));
		clear_posts();
	}

	fn menu() -> leptoskit_native::Element {
		View().class("w-full h-full").child(
			View()
				.class("absolute bottom-0 left-0 right-0 h-12 flex-row items-center px-4 bg-black")
				.child(Text("x 1").class("text-white text-base"))
				.child(
					Button("Restart")
						.class("bg-white text-black px-3")
						.on_press(|| post("restart")),
				),
		)
	}
}
