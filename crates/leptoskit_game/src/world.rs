//! Named bodies and the values a menu reads.

use std::any::Any;
use std::any::TypeId;
use std::collections::BTreeMap;

/// Steps per second. [`World::dt`] is `1 / TICK_HZ` and does not follow the
/// display.
pub const TICK_HZ: u32 = 60;

/// Where a body sits, in world units. Y is up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
	/// The body's origin.
	pub position: [f32; 3],
}

impl Default for Transform {
	fn default() -> Self {
		Self {
			position: [0.0, 0.0, 0.0],
		}
	}
}

/// A box the host draws. Scale is the full size on each axis, in world units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mesh {
	/// Full width, height, and depth.
	pub scale: [f32; 3],
	/// Straight sRGB, 8 bits per channel, alpha last.
	pub color: [u8; 4],
}

impl Default for Mesh {
	fn default() -> Self {
		Self {
			scale: [1.0, 1.0, 1.0],
			color: [255, 255, 255, 255],
		}
	}
}

/// One saved value a menu can read by name.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
	/// A number.
	Number(f64),
	/// Text.
	Text(String),
	/// A boolean.
	Flag(bool),
}

impl From<f64> for Value {
	fn from(value: f64) -> Self {
		Self::Number(value)
	}
}

impl From<f32> for Value {
	fn from(value: f32) -> Self {
		Self::Number(f64::from(value))
	}
}

impl From<bool> for Value {
	fn from(value: bool) -> Self {
		Self::Flag(value)
	}
}

impl From<String> for Value {
	fn from(value: String) -> Self {
		Self::Text(value)
	}
}

impl From<&str> for Value {
	fn from(value: &str) -> Self {
		Self::Text(value.to_owned())
	}
}

struct Entity {
	name: String,
	transform: Transform,
	mesh: Mesh,
}

struct Resource {
	id: TypeId,
	value: Box<dyn Any>,
}

/// The saved world. A [`crate::Game`] mutates it; the host only reads it to
/// draw.
#[derive(Default)]
pub struct World {
	entities: Vec<Entity>,
	resources: Vec<Resource>,
	published: BTreeMap<String, Value>,
	tick: u64,
}

impl World {
	/// An empty world at tick 0.
	pub fn new() -> Self {
		Self::default()
	}

	/// Completed ticks. The tick currently running still reports the previous
	/// count.
	pub fn tick(&self) -> u64 {
		self.tick
	}

	/// The fixed step, in seconds.
	pub fn dt(&self) -> f32 {
		1.0 / TICK_HZ as f32
	}

	pub(crate) fn advance(&mut self) {
		self.tick += 1;
	}

	/// Spawns a unit white cube, or returns the one already named `name`.
	pub fn cube(&mut self, name: impl Into<String>) -> Prop<'_> {
		let name = name.into();
		let index = self
			.entities
			.iter()
			.position(|entity| entity.name == name)
			.unwrap_or_else(|| {
				self.entities.push(Entity {
					name,
					transform: Transform::default(),
					mesh: Mesh::default(),
				});
				self.entities.len() - 1
			});
		Prop { world: self, index }
	}

	/// The position of a named body.
	pub fn position(&self, name: &str) -> Option<[f32; 3]> {
		self.entity(name).map(|entity| entity.transform.position)
	}

	/// A mutable position. `None` when `name` was never spawned.
	pub fn position_mut(&mut self, name: &str) -> Option<&mut [f32; 3]> {
		let index = self
			.entities
			.iter()
			.position(|entity| entity.name == name)?;
		Some(&mut self.entities[index].transform.position)
	}

	/// Stores `value` under `name`, replacing a previous publication.
	pub fn publish(&mut self, name: impl Into<String>, value: impl Into<Value>) {
		self.published.insert(name.into(), value.into());
	}

	/// The publication named `name`, when it is a number.
	pub fn number(&self, name: &str) -> Option<f64> {
		match self.published.get(name) {
			Some(Value::Number(value)) => Some(*value),
			_ => None,
		}
	}

	/// The publication named `name`, when it is text.
	pub fn text(&self, name: &str) -> Option<&str> {
		match self.published.get(name) {
			Some(Value::Text(value)) => Some(value),
			_ => None,
		}
	}

	/// The publication named `name`, when it is a flag.
	pub fn flag(&self, name: &str) -> Option<bool> {
		match self.published.get(name) {
			Some(Value::Flag(value)) => Some(*value),
			_ => None,
		}
	}

	/// Every number publication, in name order.
	pub fn numbers(&self) -> Vec<(String, f64)> {
		self.published
			.iter()
			.filter_map(|(name, value)| {
				match value {
					Value::Number(number) => Some((name.clone(), *number)),
					Value::Text(_) | Value::Flag(_) => None,
				}
			})
			.collect()
	}

	/// Inserts or replaces a resource. Gameplay state that is not a body lives
	/// here.
	pub fn insert<T: 'static>(&mut self, value: T) {
		let id = TypeId::of::<T>();
		if let Some(slot) = self.resources.iter_mut().find(|slot| slot.id == id) {
			slot.value = Box::new(value);
		} else {
			self.resources.push(Resource {
				id,
				value: Box::new(value),
			});
		}
	}

	/// The resource of type `T`.
	pub fn get<T: 'static>(&self) -> Option<&T> {
		let id = TypeId::of::<T>();
		self.resources
			.iter()
			.find(|slot| slot.id == id)?
			.value
			.downcast_ref()
	}

	/// A mutable resource of type `T`.
	pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
		let id = TypeId::of::<T>();
		self.resources
			.iter_mut()
			.find(|slot| slot.id == id)?
			.value
			.downcast_mut()
	}

	pub(crate) fn bodies(&self) -> impl Iterator<Item = (&Transform, &Mesh)> {
		self.entities
			.iter()
			.map(|entity| (&entity.transform, &entity.mesh))
	}

	fn entity(&self, name: &str) -> Option<&Entity> {
		self.entities.iter().find(|entity| entity.name == name)
	}
}

/// A cube that was just spawned or looked up. Place it, then colour it.
pub struct Prop<'a> {
	world: &'a mut World,
	index: usize,
}

impl Prop<'_> {
	/// Moves the cube's origin to `(x, y, z)`.
	#[must_use]
	pub fn at(self, x: f32, y: f32, z: f32) -> Self {
		self.world.entities[self.index].transform.position = [x, y, z];
		self
	}

	/// Sets the full size on each axis.
	#[must_use]
	pub fn scale(self, x: f32, y: f32, z: f32) -> Self {
		self.world.entities[self.index].mesh.scale = [x, y, z];
		self
	}

	/// Sets an opaque `#rrggbb` or an `#rrggbbaa` colour.
	///
	/// # Panics
	///
	/// Panics when `hex` is not one of those forms.
	#[must_use]
	pub fn color(self, hex: &str) -> Self {
		self.world.entities[self.index].mesh.color =
			parse_hex(hex).unwrap_or_else(|| panic!("invalid colour `{hex}`"));
		self
	}
}

/// Parses `#rrggbb` or `#rrggbbaa` into straight RGBA bytes.
pub fn parse_hex(value: &str) -> Option<[u8; 4]> {
	let digits = value.strip_prefix('#')?;
	let packed = u32::from_str_radix(digits, 16).ok()?;
	match digits.len() {
		6 => Some([(packed >> 16) as u8, (packed >> 8) as u8, packed as u8, 255]),
		8 => {
			Some([
				(packed >> 24) as u8,
				(packed >> 16) as u8,
				(packed >> 8) as u8,
				packed as u8,
			])
		}
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	struct Score(u32);

	#[test]
	fn cubes_move_publish_and_store_resources() {
		let mut world = World::new();
		assert!((world.dt() - 1.0 / 60.0).abs() < f32::EPSILON);
		let _ = world
			.cube("lantern")
			.at(1.0, 2.0, 3.0)
			.scale(2.0, 1.0, 1.0)
			.color("#e7b15a");
		let _ = world.cube("lantern").at(4.0, 2.0, 3.0);
		assert_eq!(world.position("lantern"), Some([4.0, 2.0, 3.0]));
		assert_eq!(world.bodies().count(), 1);
		world.position_mut("lantern").unwrap()[0] = 5.0;
		assert_eq!(world.position("missing"), None);
		assert!(world.position_mut("missing").is_none());

		world.publish("x", 5.0_f32);
		world.publish("label", "hi");
		world.publish("paused", true);
		world.publish("x", 6.0_f64);
		assert_eq!(world.number("x"), Some(6.0));
		assert_eq!(world.text("label"), Some("hi"));
		assert_eq!(world.flag("paused"), Some(true));
		assert_eq!(world.number("label"), None);
		assert_eq!(world.text("x"), None);
		assert_eq!(world.flag("x"), None);
		assert_eq!(world.numbers(), vec![("x".to_owned(), 6.0)]);

		world.insert(Score(1));
		world.insert(Score(2));
		assert_eq!(world.get::<Score>().map(|score| score.0), Some(2));
		world.get_mut::<Score>().unwrap().0 = 3;
		assert_eq!(world.get::<Score>().unwrap().0, 3);
		assert!(world.get::<u8>().is_none());
		world.advance();
		assert_eq!(world.tick(), 1);
	}

	#[test]
	fn hex_colours() {
		assert_eq!(parse_hex("#112233"), Some([0x11, 0x22, 0x33, 255]));
		assert_eq!(parse_hex("#11223344"), Some([0x11, 0x22, 0x33, 0x44]));
		assert_eq!(parse_hex("112233"), None);
		assert_eq!(parse_hex("#xyzxyz"), None);
		assert_eq!(parse_hex("#fff"), None);
	}

	#[test]
	#[should_panic(expected = "invalid colour")]
	fn a_bad_colour_panics() {
		let mut world = World::new();
		let _ = world.cube("a").color("nope");
	}
}
