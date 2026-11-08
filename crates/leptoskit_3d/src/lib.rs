//! <!-- {=scene3dSummary|trim|linePrefix:"//! ":true} -->
//!
//! It draws 3D scenes the host owns: glTF models with skins, morph targets, and
//! animation, lit and shaded on the GPU with wgpu, in a `Scene3d` view. A guest
//! describes a scene with compact ops and hears back through events; the host
//! animates and draws it on the display link, never in the guest's frame loop.
//!
//! <!-- {/scene3dSummary} -->
//!
//! GPU 3D scenes for leptoskit.
//!
//! A guest describes a scene with [`Op`]s and the host owns it: models loaded
//! from glTF, their animation, lights, and the camera all live with the host,
//! which draws them with wgpu on the display link, never in the guest's frame
//! loop.
//!
//! Without the default `renderer` feature, the crate is only the ops and
//! events and their binary form, and builds for any target, Wasm guests
//! included.
// Proving that `Gpu` is `Send` walks wgpu's deep types past the default
// limit.
#![recursion_limit = "256"]

pub mod client;
mod protocol;
mod sanitize;

#[cfg(feature = "renderer")]
mod animation;
#[cfg(feature = "renderer")]
mod asset;
#[cfg(feature = "renderer")]
mod environment;
#[cfg(feature = "renderer")]
pub mod gltf;

#[cfg(feature = "renderer")]
mod gpu;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub mod host;
#[cfg(feature = "renderer")]
mod image;
#[cfg(feature = "renderer")]
mod raycast;
#[cfg(feature = "renderer")]
mod renderer;
#[cfg(feature = "renderer")]
mod upload;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub mod web;
#[cfg(feature = "renderer")]
mod world;

#[cfg(feature = "renderer")]
pub use asset::Asset;
/// The math types the renderer's API takes, so hosts use the same version.
#[cfg(feature = "renderer")]
pub use glam;
#[cfg(feature = "renderer")]
pub use gltf::LoadError;
#[cfg(feature = "renderer")]
pub use gpu::Backend;
#[cfg(feature = "renderer")]
pub use gpu::Error;
#[cfg(feature = "renderer")]
pub use gpu::Gpu;
pub use protocol::*;
#[cfg(feature = "renderer")]
pub use renderer::*;
pub use sanitize::*;
/// The wgpu the renderer is built on, so hosts create devices, surfaces, and
/// textures with the same version.
#[cfg(feature = "renderer")]
pub use wgpu;
#[cfg(feature = "renderer")]
pub use world::World;

#[cfg(feature = "renderer")]
impl Asset {
	/// Loads a model from the bytes of a `.glb` file, or of a `.gltf` file
	/// whose buffers and images are embedded as `data:` URIs.
	pub fn from_slice(bytes: &[u8]) -> Result<Self, LoadError> {
		gltf::load(bytes, &mut |path| {
			Err(format!("{path} is not embedded in the model"))
		})
	}

	/// Loads a model whose `.gltf` refers to files beside it. `resolve` reads
	/// one by its relative path, which is never outside the model's folder.
	pub fn from_slice_with(
		bytes: &[u8],
		mut resolve: impl FnMut(&str) -> Result<Vec<u8>, String>,
	) -> Result<Self, LoadError> {
		gltf::load(bytes, &mut resolve)
	}
}
