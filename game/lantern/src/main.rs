//! A lantern that drifts, and a menu that can put it back.
//!
//! Gameplay is the `Game` impl. The bar along the bottom is a leptoskit tree.

use leptoskit::game::Game;
use leptoskit::game::Input;
use leptoskit::game::World;
use leptoskit::game::post;
use leptoskit::prelude::*;

struct Lantern;

impl Game for Lantern {
	const ID: &'static str = "lantern";

	fn setup(world: &mut World) {
		let _ = world
			.cube("ground")
			.at(0.0, -0.05, 0.0)
			.scale(6.0, 0.1, 6.0)
			.color("#2f3d34");
		let _ = world
			.cube("lantern")
			.at(0.0, 0.5, 0.0)
			.scale(0.55, 0.9, 0.55)
			.color("#e7b15a");
	}

	fn tick(world: &mut World, input: &Input) {
		let restart = input.messages().iter().any(|message| message == "restart");
		let speed = if input.held("move") { 30.0 } else { 6.0 };
		let step = speed * world.dt();
		let x = {
			let Some(position) = world.position_mut("lantern") else {
				return;
			};
			if restart {
				position[0] = 0.0;
			} else {
				position[0] += step;
			}
			f64::from(position[0])
		};
		world.publish("x", x);
	}
}

fn hud(world: &World) -> Element {
	let x = world.number("x").unwrap_or(0.0);
	View().class("w-full h-full").child(
		View()
			.class(
				"absolute bottom-0 left-0 right-0 h-14 flex-row items-center justify-between px-4 \
				 bg-black",
			)
			.child(Text(format!("x {x:.1}")).class("text-white text-base"))
			.child(
				Button("Restart")
					.class("bg-white text-black px-3")
					.on_press(|| post("restart")),
			),
	)
}

fn main() {
	if let Err(error) = leptoskit::game::run::<Lantern, _>(hud) {
		eprintln!("lantern: {error}");
		std::process::exit(1);
	}
}

#[cfg(test)]
mod tests {
	use leptoskit::game::Game;
	use leptoskit::game::Sim;
	use leptoskit::game::World;
	use leptoskit::game::clear_posts;
	use leptoskit::game::post;
	use leptoskit::prelude::NativeKind;

	use super::Lantern;
	use super::hud;

	#[test]
	fn drift_then_restart() {
		clear_posts();
		let mut sim = Sim::<Lantern>::new();
		sim.step();
		let moved = sim.world().number("x").unwrap();
		assert!(moved > 0.0);
		post("restart");
		sim.step();
		assert_eq!(sim.world().number("x"), Some(0.0));
		sim.hold("move");
		sim.step();
		assert!(sim.world().number("x").unwrap() > moved);
	}

	#[test]
	fn the_menu_is_a_leptoskit_tree() {
		let mut world = World::new();
		Lantern::setup(&mut world);
		let element = hud(&world);
		assert_eq!(element.kind, NativeKind::View);
	}
}
