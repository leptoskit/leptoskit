//! The guest's side of a scene: queue ops, send them to the host, and wait
//! for its events.
//!
//! In a native app the ops travel through the `leptoskit.scene3d` plugin to
//! the host, which draws on its own display link. In the browser they go to
//! the page's own renderer. Either way the app's code is the same.

use std::cell::Cell;
use std::cell::RefCell;

use crate::protocol::*;

/// Why the host did not take a batch or answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientError {
	/// The host has no 3D support: the app was built without it.
	Unavailable,
	/// The host refused the batch as malformed, or it holds the most scenes
	/// it may.
	Rejected,
	/// The call was cancelled or the bridge failed.
	Failed,
}

impl std::fmt::Display for ClientError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str(match self {
			Self::Unavailable => "this app was built without 3D support",
			Self::Rejected => "the host refused the scene's ops",
			Self::Failed => "the scene's host failed to answer",
		})
	}
}

impl std::error::Error for ClientError {}

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
mod transport {
	use leptoskit_bridge::guest::Client;

	use super::ClientError;
	use crate::protocol::*;

	const PLUGIN: &str = "leptoskit.scene3d";
	const APPLY: leptoskit_bridge::MethodId = leptoskit_bridge::MethodId(1);
	const NEXT_EVENTS: leptoskit_bridge::MethodId = leptoskit_bridge::MethodId(2);
	const CLOSE: leptoskit_bridge::MethodId = leptoskit_bridge::MethodId(3);

	fn error(error: leptoskit_bridge::Error) -> ClientError {
		match error {
			leptoskit_bridge::Error::Unavailable => ClientError::Unavailable,
			leptoskit_bridge::Error::InvalidArgument | leptoskit_bridge::Error::TooLarge => {
				ClientError::Rejected
			}
			_ => ClientError::Failed,
		}
	}

	fn client() -> Result<Client, ClientError> {
		Client::new(PLUGIN).map_err(error)
	}

	fn payload(scene: SceneId, rest: &[u8]) -> Vec<u8> {
		let mut payload = Vec::with_capacity(4 + rest.len());
		payload.extend_from_slice(&scene.0.to_le_bytes());
		payload.extend_from_slice(rest);
		payload
	}

	/// Submits the batch now, so batches reach the host in the order they
	/// were sent; the future only waits for the host's answer.
	pub fn send(
		scene: SceneId,
		ops: &[Op],
	) -> Result<impl Future<Output = Result<(), ClientError>> + use<>, ClientError> {
		let call = client()?
			.call(APPLY, &payload(scene, &encode_ops(ops)))
			.map_err(error)?;
		Ok(async move { call.await.map(|_| ()).map_err(error) })
	}

	pub async fn next_events(scene: SceneId) -> Result<Vec<Event>, ClientError> {
		let reply = client()?
			.call(NEXT_EVENTS, &payload(scene, &[]))
			.map_err(error)?
			.await
			.map_err(error)?;
		decode_events(&reply).map_err(|_| ClientError::Failed)
	}

	pub fn close(
		scene: SceneId,
	) -> Result<impl Future<Output = Result<(), ClientError>>, ClientError> {
		let call = client()?.call(CLOSE, &payload(scene, &[])).map_err(error)?;
		Ok(async move { call.await.map(|_| ()).map_err(error) })
	}
}

#[cfg(all(not(feature = "guest"), feature = "web", target_arch = "wasm32"))]
mod transport {
	use super::ClientError;
	use crate::protocol::*;

	// The same signatures as the guest's transport, which can fail to submit.
	#[allow(clippy::unnecessary_wraps)]
	pub fn send(
		scene: SceneId,
		ops: &[Op],
	) -> Result<impl Future<Output = Result<(), ClientError>> + use<>, ClientError> {
		crate::web::apply(scene, ops.to_vec());
		Ok(std::future::ready(Ok(())))
	}

	pub async fn next_events(scene: SceneId) -> Result<Vec<Event>, ClientError> {
		Ok(crate::web::next_events(scene).await)
	}

	#[allow(clippy::unnecessary_wraps)]
	pub fn close(
		scene: SceneId,
	) -> Result<impl Future<Output = Result<(), ClientError>>, ClientError> {
		crate::web::close(scene);
		Ok(std::future::ready(Ok(())))
	}
}

/// A scene the guest describes and the host draws.
///
/// Builder methods queue ops and hand back the ids they chose; nothing
/// reaches the host until [`Scene::flush`]. A `Scene3d` view with the same
/// [`SceneId`] shows it.
///
/// ```no_run
/// # async fn show() -> Result<(), leptoskit_3d::client::ClientError> {
/// use leptoskit_3d::Play;
/// use leptoskit_3d::SceneId;
/// use leptoskit_3d::Transform;
/// use leptoskit_3d::client::Scene;
///
/// let scene = Scene::new(SceneId(1));
/// let reaper = scene.load("models/reaper.glb");
/// let figure = scene.spawn(reaper);
/// scene.set_transform(figure, Transform::from_translation(0.0, 0.0, -2.0));
/// scene.play(figure, Play::new("idle").fade(0.2));
/// # #[cfg(target_arch = "wasm32")]
/// scene.flush().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Scene {
	id: SceneId,
	queue: RefCell<Vec<Op>>,
	next_id: Cell<u32>,
}

impl Scene {
	/// A scene with `id`, empty until its first flush.
	pub fn new(id: SceneId) -> Self {
		Self {
			id,
			queue: RefCell::new(Vec::new()),
			next_id: Cell::new(1),
		}
	}

	/// The scene's id, for the `Scene3d` view that shows it.
	pub fn id(&self) -> SceneId {
		self.id
	}

	fn allocate(&self) -> u32 {
		let id = self.next_id.get();
		self.next_id.set(id.wrapping_add(1).max(1));
		id
	}

	/// Queues an op.
	pub fn push(&self, op: Op) {
		self.queue.borrow_mut().push(op);
	}

	/// How many ops wait for the next flush.
	pub fn pending(&self) -> usize {
		self.queue.borrow().len()
	}

	/// Loads a model from the app's assets, such as `models/reaper.glb`.
	pub fn load(&self, path: impl Into<String>) -> AssetId {
		let asset = AssetId(self.allocate());
		self.push(Op::LoadAsset {
			asset,
			path: path.into(),
		});
		asset
	}

	/// Adds an instance of `asset` at the scene's root.
	pub fn spawn(&self, asset: AssetId) -> NodeId {
		self.spawn_in(Some(asset), None)
	}

	/// Adds an empty group at the scene's root, to move nodes together.
	pub fn group(&self) -> NodeId {
		self.spawn_in(None, None)
	}

	/// Adds an instance of `asset` (or a group) under `parent`.
	pub fn spawn_in(&self, asset: Option<AssetId>, parent: Option<NodeId>) -> NodeId {
		let node = NodeId(self.allocate());
		self.push(Op::Spawn {
			node,
			asset,
			parent,
		});
		node
	}

	/// Places a node relative to its parent.
	pub fn set_transform(&self, node: NodeId, transform: Transform) {
		self.push(Op::SetTransform { node, transform });
	}

	/// Shows or hides a node.
	pub fn set_visible(&self, node: NodeId, visible: bool) {
		self.push(Op::SetVisible { node, visible });
	}

	/// Shows or hides a named part of a model instance.
	pub fn set_part_visible(&self, node: NodeId, part: impl Into<String>, visible: bool) {
		self.push(Op::SetPartVisible {
			node,
			part: part.into(),
			visible,
		});
	}

	/// Plays a clip.
	pub fn play(&self, node: NodeId, play: Play) {
		self.push(Op::Play { node, play });
	}

	/// Stops the clip playing, fading over `fade` seconds.
	pub fn stop(&self, node: NodeId, fade: f32) {
		self.push(Op::Stop { node, fade });
	}

	/// Sets a morph target's weight on every mesh of the instance that has
	/// it.
	pub fn set_morph_weight(&self, node: NodeId, target: impl Into<String>, weight: f32) {
		self.push(Op::SetMorphWeight {
			node,
			part: None,
			target: target.into(),
			weight,
		});
	}

	/// Changes how a part (or, with `None`, the whole instance) draws.
	pub fn set_material(&self, node: NodeId, part: Option<&str>, material: MaterialOverride) {
		self.push(Op::SetMaterial {
			node,
			part: part.map(str::to_owned),
			material,
		});
	}

	/// Makes `node` follow a named part of the instance `target`.
	pub fn attach(&self, node: NodeId, target: NodeId, bone: impl Into<String>) {
		self.push(Op::Attach {
			node,
			target,
			bone: bone.into(),
		});
	}

	/// Removes a node and everything under it.
	pub fn remove(&self, node: NodeId) {
		self.push(Op::Remove { node });
	}

	/// Asks what is under a point of the view (`x` and `y` from 0 to 1, from
	/// the top-left); [`Event::RaycastHits`] with the returned request
	/// answers.
	pub fn raycast_screen(&self, x: f32, y: f32) -> RequestId {
		let request = RequestId(self.allocate());
		self.push(Op::RaycastScreen { request, x, y });
		request
	}

	/// Sends the queued ops to the host and waits until it has applied
	/// them. The batch is sent before the first poll, so batches arrive in
	/// the order `flush` was called.
	#[cfg(all(any(feature = "guest", feature = "web"), target_arch = "wasm32"))]
	pub fn flush(&self) -> impl Future<Output = Result<(), ClientError>> + use<> {
		let ops = std::mem::take(&mut *self.queue.borrow_mut());
		let sent = if ops.is_empty() {
			None
		} else {
			Some(
				ops.chunks(crate::MAX_OPS)
					.map(|chunk| transport::send(self.id, chunk))
					.collect::<Result<Vec<_>, _>>(),
			)
		};

		async move {
			for call in sent.transpose()?.into_iter().flatten() {
				call.await?;
			}

			Ok(())
		}
	}

	/// Waits for the host's next events about this scene.
	#[cfg(all(any(feature = "guest", feature = "web"), target_arch = "wasm32"))]
	pub async fn next_events(&self) -> Result<Vec<Event>, ClientError> {
		transport::next_events(self.id).await
	}

	/// Asks the host to forget the scene.
	#[cfg(all(any(feature = "guest", feature = "web"), target_arch = "wasm32"))]
	pub fn close(self) -> impl Future<Output = Result<(), ClientError>> {
		let call = transport::close(self.id);
		async move { call?.await }
	}

	/// Takes the queued ops without sending them, for hosts that apply them
	/// in process.
	pub fn take_ops(&self) -> Vec<Op> {
		std::mem::take(&mut *self.queue.borrow_mut())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn builders_queue_ops_with_fresh_ids() {
		let scene = Scene::new(SceneId(3));
		let asset = scene.load("models/rig.glb");
		let node = scene.spawn(asset);
		let group = scene.group();
		scene.attach(node, group, "");
		scene.play(node, Play::new("idle"));
		let request = scene.raycast_screen(0.5, 0.5);
		assert_eq!(scene.pending(), 6);

		assert_ne!(asset.0, node.0);
		assert_ne!(node, group);
		assert_eq!(request.0, 4);
		let ops = scene.take_ops();
		assert_eq!(
			ops[1],
			Op::Spawn {
				node,
				asset: Some(asset),
				parent: None,
			}
		);
		assert_eq!(scene.pending(), 0);
		assert_eq!(scene.id(), SceneId(3));
	}
}
