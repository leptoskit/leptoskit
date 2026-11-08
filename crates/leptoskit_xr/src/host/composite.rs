//! Several backends behind one: each mode goes to the first backend that
//! offers it, so a phone pairs ARCore with the rotation sensors and a headset
//! build pairs OpenXR with them.

use leptoskit_bridge::Error;

use super::Backend;
use super::Viewport;
use super::XrFrame;
use crate::math::Pose;
use crate::types::*;

/// Routes each session to the first backend that offers its mode.
#[derive(Default)]
pub struct Composite {
	backends: Vec<Box<dyn Backend>>,
	active: Option<usize>,
}

impl std::fmt::Debug for Composite {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Composite")
			.field("backends", &self.backends.len())
			.field("active", &self.active)
			.finish()
	}
}

impl Composite {
	/// No backends: a device that can do nothing.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Adds a backend after the ones already added, which win for modes they
	/// share.
	#[must_use]
	pub fn with(mut self, backend: impl Backend + 'static) -> Self {
		self.backends.push(Box::new(backend));
		self
	}

	fn owner(&self, mode: Mode) -> Option<usize> {
		self.backends
			.iter()
			.position(|backend| backend.capabilities().supports(mode))
	}

	fn active(&mut self) -> Result<&mut Box<dyn Backend>, Error> {
		let index = self.active.ok_or(Error::Unavailable)?;
		Ok(&mut self.backends[index])
	}
}

impl Backend for Composite {
	fn capabilities(&self) -> Capabilities {
		Mode::ALL
			.iter()
			.fold(Capabilities::NONE, |capabilities, &mode| {
				match self.owner(mode) {
					Some(index) => {
						capabilities.with(
							mode,
							self.backends[index]
								.capabilities()
								.features(mode)
								.unwrap_or_default(),
						)
					}
					None => capabilities,
				}
			})
	}

	fn prepare(&mut self, mode: Mode, install: bool) -> Availability {
		match self.owner(mode) {
			Some(index) => self.backends[index].prepare(mode, install),
			None => Availability::Unsupported,
		}
	}

	fn start(&mut self, config: &SessionConfig, features: Features) -> Result<(), Error> {
		let index = self.owner(config.mode).ok_or(Error::Unavailable)?;
		self.backends[index].start(config, features)?;
		self.active = Some(index);
		Ok(())
	}

	fn configure(&mut self, config: &SessionConfig, features: Features) -> Result<(), Error> {
		self.active()?.configure(config, features)
	}

	fn update(&mut self, time: f64) -> Result<Option<XrFrame>, Error> {
		self.active()?.update(time)
	}

	fn hit_test(&mut self, request: &HitTestRequest, frame: &XrFrame) -> Vec<Hit> {
		match self.active() {
			Ok(backend) => backend.hit_test(request, frame),
			Err(_) => Vec::new(),
		}
	}

	fn add_anchor(&mut self, pose: Pose, plane: Option<u64>) -> Result<u64, Error> {
		self.active()?.add_anchor(pose, plane)
	}

	fn remove_anchor(&mut self, id: u64) -> Result<(), Error> {
		self.active()?.remove_anchor(id)
	}

	fn set_viewport(&mut self, viewport: Viewport) {
		for backend in &mut self.backends {
			backend.set_viewport(viewport);
		}
	}

	fn stop(&mut self) {
		if let Some(index) = self.active.take() {
			self.backends[index].stop();
		}
	}
}
