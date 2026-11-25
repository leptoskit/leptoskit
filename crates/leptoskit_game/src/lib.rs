//! A high-performance 3D and 2D game engine for leptoskit.
//!
//! Gameplay stays pure Rust, powered by a cache-friendly, safe-Rust Archetype
//! ECS ([`ecs`]), modular [`GameApp`] schedules and [`GamePlugin`]s,
//! double-buffered [`Events`], and Godot-style names, groups, and synchronous
//! signal observers.
//!
//! Menus and HUDs are ordinary leptoskit components (`View`, `Text`, `Button`,
//! Tailwind classes). Each frame the host commits that tree through
//! [`leptoskit_native::NativeSurface`], the same mutation plan the native
//! backends apply, and draws the resulting interface over the game world.
//! There is no second UI language and no second compiler.
//!
//! Quick-start gameplay can also implement [`Game`] with [`Game::setup`] and
//! [`Game::tick`]; messages a button posts with [`post`] arrive on the
//! following tick.

pub mod ecs;
#[cfg(not(target_arch = "wasm32"))]
mod font;
#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
mod overlay;
mod project;
#[cfg(not(target_arch = "wasm32"))]
mod raster;
#[cfg(not(target_arch = "wasm32"))]
mod scene;
mod sim;
mod world;

pub use ecs::Added;
pub use ecs::ArchetypeId;
pub use ecs::Bundle;
pub use ecs::Changed;
pub use ecs::Commands;
pub use ecs::Component;
pub use ecs::ComponentId;
pub use ecs::ComponentTicks;
pub use ecs::Entity;
pub use ecs::EventCursor;
pub use ecs::EventId;
pub use ecs::Events;
pub use ecs::GameApp;
pub use ecs::GamePlugin;
pub use ecs::Mut;
pub use ecs::NextState;
pub use ecs::ObserverId;
pub use ecs::QueryFilter;
pub use ecs::QueryPlan;
pub use ecs::ReadQuery;
pub use ecs::Schedule;
pub use ecs::Stage;
pub use ecs::State;
pub use ecs::SystemAccess;
pub use ecs::SystemConfig;
pub use ecs::Time;
pub use ecs::Trigger;
pub use ecs::With;
pub use ecs::Without;
pub use ecs::WriteQuery;
#[cfg(not(target_arch = "wasm32"))]
pub use host::Options;
#[cfg(not(target_arch = "wasm32"))]
pub use host::Painter;
#[cfg(not(target_arch = "wasm32"))]
pub use host::Report;
#[cfg(not(target_arch = "wasm32"))]
pub use host::run;
#[cfg(not(target_arch = "wasm32"))]
pub use host::run_with;
pub use project::Camera;
pub use sim::Game;
pub use sim::Input;
pub use sim::Sim;
pub use sim::clear_posts;
pub use sim::post;
pub use world::Mesh;
pub use world::Prop;
pub use world::TICK_HZ;
pub use world::Transform;
pub use world::Value;
pub use world::World;
pub use world::parse_hex;

/// A failure while parsing arguments, painting, or writing a frame.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// A zero frame count or a zero edge.
	#[error("frames, width, and height must be greater than zero")]
	Empty,
	/// The frame is larger than the host allows.
	#[error("width and height must be at most 4096")]
	TooLarge,
	/// `--tap` needs a later tick to deliver the press.
	#[error("`--tap` needs at least two frames so the press reaches a tick")]
	TapFrames,
	/// An argument this host does not recognise.
	#[error("unknown argument `{0}`")]
	Argument(String),
	/// A flag that needs a value was the last argument.
	#[error("missing value for `{0}`")]
	Missing(&'static str),
	/// A numeric flag was not a whole number.
	#[error("`{flag}` expected a whole number, got `{value}`")]
	Number {
		/// The flag.
		flag: &'static str,
		/// The text that was passed.
		value: String,
	},
	/// `--painter` was not `auto`, `gpu`, or `cpu`.
	#[error("unknown painter `{0}` (expected auto, gpu, or cpu)")]
	Painter(String),
	/// No committed control has this text or accessibility label.
	#[error("no control labeled `{0}`")]
	Label(String),
	/// The control has no clickable frame.
	#[error("the control labeled `{0}` has no frame")]
	Frame(String),
	/// wgpu refused to draw, and the caller asked for the GPU painter only.
	#[error("gpu painter failed: {0}")]
	Gpu(String),
	/// The PNG could not be created.
	#[error("could not write {path}: {source}")]
	Write {
		/// The destination.
		path: std::path::PathBuf,
		/// The filesystem error.
		#[source]
		source: std::io::Error,
	},
	/// The PNG encoder rejected the pixels.
	#[error("could not encode {path}: {message}")]
	Encode {
		/// The destination.
		path: std::path::PathBuf,
		/// The encoder's message.
		message: String,
	},
}
