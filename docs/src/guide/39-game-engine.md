# A game engine

Use a game host when the screen is a world that moves on its own clock, with menus and a HUD in front of it. A character walking across a floor, a score, a button that starts the round again. The world is Rust. The menus are the same leptoskit components the rest of the app uses.

The host commits that component tree through the native renderer, the same mutation plan macOS, iOS, Android, Windows, Linux, and the terminal apply, and draws the resulting boxes over a wgpu picture of the world. Gameplay never goes through that plan. There is no second language to compile.

This runs as a desktop binary on Linux, macOS, and Windows. It does not build an iOS, Android, or browser app, and it does not open a window: it writes the last frame as a PNG and prints the numbers the menu published. The picture uses Vulkan on Linux, Metal on macOS, and Direct3D 12 on Windows. A machine with no GPU adapter paints the same frame on the CPU, and the report says `painter: cpu`. `View`, `Text`, and `Button` are drawn. Other widgets still commit and take a frame, but only their background colour and text are painted.

Enable the `game` feature and implement `Game`. `setup` builds the world once. `tick` advances one fixed step, sixty to a second, which does not follow the display. A button calls `post`; the message arrives on the next tick.

```rust
use leptoskit::game::post;
use leptoskit::game::Game;
use leptoskit::game::Input;
use leptoskit::game::World;
use leptoskit::prelude::*;

struct Lantern;

impl Game for Lantern {
	const ID: &'static str = "lantern";

	fn setup(world: &mut World) {
		world.cube("lantern").at(0.0, 0.5, 0.0).color("#e7b15a");
	}

	fn tick(world: &mut World, input: &Input) {
		let restart = input.messages().iter().any(|message| message == "restart");
		let step = if input.held("move") { 30.0 * world.dt() } else { 0.0 };
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
	view! {
		<View class="w-full h-full">
			<View class="absolute bottom-0 left-0 right-0 h-14 flex-row items-center justify-between px-4 bg-black">
				<Text class="text-white text-base">{format!("x {x:.1}")}</Text>
				<Button on_press=|| post("restart")>"Restart"</Button>
			</View>
		</View>
	}
}
```

`game/lantern` is a small game in that shape: a lantern drifts, and Restart puts it back. From the repository:

```sh
leptoskit game run --frames 12 --painter cpu --out artifacts/frame.png
```

The report names the painter, the tick, and each number the world published. `--hold move` keeps an action down. `--tap Restart` presses that label halfway through, which needs at least two frames so a later tick can read it. `--painter gpu` refuses to fall back when no adapter is available.

`leptoskit game build` only compiles the crate. Pass a directory to build a game that is not `game/lantern`.
